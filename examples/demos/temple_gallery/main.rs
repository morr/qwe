//! Витрина храмов и крепости: по ряду на вероисповедание, в ряду — формы
//! храма, какие встречаются в OSM, и последним рядом кремль.
//!
//! **Всё здесь строит игра.** Слои спавнит `spawn_buildings` — тот же вызов,
//! что поднимает город: стены, кровли, главы, шпили, минареты, зубцы и тени.
//! Витрина задаёт только контуры и теги (назначение с верой, крепость), то есть
//! ровно то, что игре приходит из OSM. Кровлю, облицовку, раскладку глав и их
//! цвета выбирает сама игра.
//!
//! Ряды — `Faith`, колонки — что за контур:
//!
//! - **корабль** 44 × 18 м — вытянутый храм с трапезной: у православного по
//!   оси встаёт колокольня, у западного — башня со шпилем;
//! - **четверик** 26 × 24 м — большой храм без вытянутости: пять глав или
//!   одна, у мечети — купол с двумя минаретами;
//! - **часовня** 10 × 9 м;
//! - **колокольня** 8 × 8 м, 30 м — `tower:type=bell_tower`;
//! - **глава** 9 × 9 м — часть собора с `roof:shape=onion`.
//!
//! Последний ряд — **кремль**: прясло стены, квадратная и восьмигранная башни.
//!
//! ```text
//! cargo run --example temple_gallery
//! ```
//!
//! `TEMPLE_GALLERY_SHOT=путь.png` — снять витрину и выйти.

#[path = "../gallery_shot.rs"]
mod shot;

use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::sprite_render::Material2dPlugin;
use qwe::camera::{hovering_ui, zoom_to_cursor};
use qwe::map::buildings::material::{
    RoofMaterial, RoofMaterialHandle, init_roof_material, retune_roof_material,
};
use qwe::map::buildings::{BuildingHeightMode, BuildingPlan, BuildingZoomBucket, spawn_buildings};
use qwe::map::osm::{AreaKind, BuildingUse, Colours, Faith, PolyArea, Sacred, SacredForm};
use qwe::map::{GROUND_COLOR, RoofStyle, SunOnMap, apply_sun};

use crate::shot::{ShotRequest, auto_shot, request_shot};

const WINDOW_WIDTH: f32 = 1600.0;
const WINDOW_HEIGHT: f32 = 1000.0;

/// Шаг сетки, м: колонка вмещает корабль с тенью, ряд — колокольню с шатром.
const PITCH: Vec2 = Vec2::new(78.0, 62.0);
/// Подпись под контуром, м ниже его центра.
const CAPTION_DROP: f32 = 20.0;
const TEXT_SCALE: f32 = 0.1;
const CAPTION_FONT: f32 = 26.0;
const HEADER_FONT: f32 = 34.0;
/// Колонка подписей рядов слева, м.
const ROW_LABEL_WIDTH: f32 = 60.0;

const FAITHS: [(Faith, &str); 5] = [
    (Faith::Orthodox, "Православный"),
    (Faith::Western, "Западный"),
    (Faith::Muslim, "Мечеть"),
    (Faith::Jewish, "Синагога"),
    (Faith::Eastern, "Восточный"),
];

/// Колонка: подпись, полуразмеры контура, высота, форма части храма.
struct Column {
    label: &'static str,
    half: Vec2,
    height: f32,
    form: SacredForm,
}

const COLUMNS: [Column; 5] = [
    Column {
        label: "корабль 44×18",
        half: Vec2::new(22.0, 9.0),
        height: 14.0,
        form: SacredForm::Nave,
    },
    Column {
        label: "четверик 26×24",
        half: Vec2::new(13.0, 12.0),
        height: 16.0,
        form: SacredForm::Nave,
    },
    Column {
        label: "часовня 10×9",
        half: Vec2::new(5.0, 4.5),
        height: 7.0,
        form: SacredForm::Nave,
    },
    Column {
        label: "колокольня 8×8",
        half: Vec2::new(4.0, 4.0),
        height: 30.0,
        form: SacredForm::Tower,
    },
    Column {
        label: "глава 9×9",
        half: Vec2::new(4.5, 4.5),
        height: 20.0,
        form: SacredForm::Dome,
    },
];

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe temple gallery — храмы и кремль".to_string(),
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
        .add_plugins(Material2dPlugin::<RoofMaterial>::default())
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<RoofStyle>()
        .init_resource::<SunOnMap>()
        .insert_resource(ClearColor(GROUND_COLOR))
        .add_systems(
            Startup,
            (
                spawn_camera,
                (apply_sun, init_roof_material).chain(),
                request_shot("TEMPLE_GALLERY_SHOT"),
            ),
        )
        .add_systems(
            Update,
            (
                zoom_to_cursor.run_if(not(hovering_ui)),
                retune_roof_material.run_if(resource_changed::<RoofStyle>),
                build.run_if(run_once),
                auto_shot.run_if(resource_exists::<ShotRequest>),
            ),
        )
        .run();
}

fn grid_size() -> Vec2 {
    Vec2::new(
        ROW_LABEL_WIDTH + PITCH.x * COLUMNS.len() as f32,
        PITCH.y * (FAITHS.len() + 1) as f32,
    )
}

/// `TEMPLE_GALLERY_FOCUS=ряд,колонка,метров_на_пиксель` — открыть витрину
/// крупным планом на одной клетке (ряд 5 — кремль): снимку из сессии иначе не
/// дотянуться до маковки.
fn focus() -> Option<(Vec2, f32)> {
    let value = std::env::var("TEMPLE_GALLERY_FOCUS").ok()?;
    let parts: Vec<f32> = value
        .split(',')
        .filter_map(|part| part.parse().ok())
        .collect();
    let [row, column, zoom] = parts[..] else {
        return None;
    };
    let center = Vec2::new(
        ROW_LABEL_WIDTH + PITCH.x * (column + 0.5),
        PITCH.y * (FAITHS.len() as f32 - row),
    );
    Some((center, zoom))
}

fn spawn_camera(mut commands: Commands, window: Single<&Window>) {
    let size = grid_size();
    let zoom = (size.x * 1.05 / window.width()).max(size.y * 1.05 / window.height());
    let (center, zoom) = focus().unwrap_or((size * 0.5 - Vec2::new(0.0, PITCH.y * 0.5), zoom));
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: zoom,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(center.extend(100.0)),
        PanCamera::default(),
    ));
}

/// Контур-прямоугольник вокруг центра, против часовой.
fn rect(center: Vec2, half: Vec2) -> Vec<Vec2> {
    vec![
        center + Vec2::new(-half.x, -half.y),
        center + Vec2::new(half.x, -half.y),
        center + Vec2::new(half.x, half.y),
        center + Vec2::new(-half.x, half.y),
    ]
}

/// Правильный многоугольник — восьмигранная башня.
fn polygon(center: Vec2, radius: f32, sides: usize) -> Vec<Vec2> {
    (0..sides)
        .map(|side| {
            center + Vec2::from_angle(std::f32::consts::TAU * side as f32 / sides as f32) * radius
        })
        .collect()
}

fn area(outer: Vec<Vec2>, height: f32, kind: AreaKind, building_use: BuildingUse) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use,
        height: Some(height),
        entrances: Vec::new(),
        colours: Colours::default(),
    }
}

fn build(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    roof: Res<RoofMaterialHandle>,
    assets: Res<AssetServer>,
) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    let caption = |commands: &mut Commands, text: String, at: Vec2, size: f32, anchor: Anchor| {
        commands.spawn((
            Text2d::new(text),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(Color::srgb(0.14, 0.16, 0.20)),
            anchor,
            Transform::from_translation(at.extend(20.0)).with_scale(Vec3::splat(TEXT_SCALE)),
        ));
    };

    let mut buildings = Vec::new();
    // сверху вниз: первая вера — верхний ряд
    for (row, (faith, label)) in FAITHS.iter().enumerate() {
        let y = PITCH.y * (FAITHS.len() - row) as f32;
        caption(
            &mut commands,
            label.to_string(),
            Vec2::new(4.0, y),
            HEADER_FONT,
            Anchor::CENTER_LEFT,
        );
        for (index, column) in COLUMNS.iter().enumerate() {
            let center = Vec2::new(ROW_LABEL_WIDTH + PITCH.x * (index as f32 + 0.5), y);
            let sacred = Sacred {
                faith: *faith,
                form: column.form,
                complex: 0,
                floor_dm: 0,
            };
            buildings.push(area(
                rect(center, column.half),
                column.height,
                AreaKind::Building,
                BuildingUse::Church(sacred),
            ));
            if row == 0 {
                caption(
                    &mut commands,
                    column.label.to_string(),
                    center + Vec2::new(0.0, PITCH.y * 0.62),
                    CAPTION_FONT,
                    Anchor::CENTER,
                );
            }
        }
    }

    // кремль: прясло, квадратная и восьмигранная башни
    let y = 0.0;
    caption(
        &mut commands,
        "Кремль".to_string(),
        Vec2::new(4.0, y),
        HEADER_FONT,
        Anchor::CENTER_LEFT,
    );
    let wall_center = Vec2::new(ROW_LABEL_WIDTH + PITCH.x, y);
    buildings.push(area(
        rect(wall_center, Vec2::new(52.0, 1.6)),
        12.7,
        AreaKind::Kremlin,
        BuildingUse::Other,
    ));
    buildings.push(area(
        rect(wall_center + Vec2::new(58.0, 0.0), Vec2::splat(5.5)),
        30.0,
        AreaKind::Kremlin,
        BuildingUse::Other,
    ));
    buildings.push(area(
        polygon(wall_center + Vec2::new(-58.0, 0.0), 6.5, 8),
        26.0,
        AreaKind::Kremlin,
        BuildingUse::Other,
    ));
    caption(
        &mut commands,
        "прясло стены 104×3.2, башни".to_string(),
        wall_center + Vec2::new(0.0, -CAPTION_DROP),
        CAPTION_FONT,
        Anchor::CENTER,
    );

    spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        &roof,
        BuildingPlan {
            mode: BuildingHeightMode::ExtrusionShadowsTint,
            bucket: BuildingZoomBucket::for_zoom(0.0),
            shadows: true,
        },
        &buildings,
        &[],
    );
}
