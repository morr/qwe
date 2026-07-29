//! Слой дорог, аллей и стен Кремля: по ленте на `RoadLine`/`WallLine`, слитой
//! в merged-меш на класс. Стиль ленты — ресурс [`RoadStyle`], переключаемый на
//! лету панелью Roads (`ui/roads.rs`); правка пересобирает только эти слои
//! ([`rebuild_roads`]).
//!
//! Раньше дороги рисовал `MeshBuilder::push_polyline` — свой квад на
//! сегмент, продлённый с обоих концов на полуширины. Стыков у него нет вообще:
//! на изломе продление торчит за внешний угол прямоугольным выступом, между
//! двумя выступами остаётся выемка, а торец пути — квадратный шип. Дефолт
//! теперь [`RoadJoin::Round`] — дуга на внешней стороне излома и полудиск на
//! торце, то же самое, что `stroke-linejoin: round` + `stroke-linecap: round`
//! у Mapnik, которым нарисован osm-carto: круглые торцы двух ways в общем узле
//! перекрываются и сливаются в скруглённый стык.
//!
//! Осевая (`RoadLine::points`) при этом **не трогается**: на ней стоят навмеш
//! (`bridge`/`passage`-прорезы), арки, посадка деревьев и генератор дверей.
//! Chaikin-сглаживание работает на копии и только ради картинки.

use std::borrow::Cow;
use std::f32::consts::PI;
use std::ops::RangeInclusive;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::loading::AppState;
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::{MapData, RoadClass, RoadLine, WallLine};
use crate::settings::{Z_ALLEY, Z_ALLEY_CASING, Z_BUILDING, Z_ROAD, Z_ROAD_CASING};

const ROAD_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Кант дороги — затемнённая заливка, как у osm-carto (белая улица в сером
/// канте). Отдельным слоем под заливкой: заливки всех дорог кроют канты всех
/// дорог, поэтому кант никогда не режет перекрёсток пополам.
const ROAD_CASING_COLOR: Color = Color::srgb(0.702, 0.702, 0.702);
const ALLEY_CASING_COLOR: Color = Color::srgb(0.729, 0.678, 0.549);

/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

/// Толщина канта — доля ширины дороги. Границы: у аллеи (3.5 м) кант обязан
/// быть виден, у магистрали (16 м) — не превратиться во вторую дорогу.
const CASING_SCALE: f32 = 0.08;
const CASING_RANGE: RangeInclusive<f32> = 0.3..=1.0;

/// Изломы мельче Chaikin не срезает: прямые участки обязаны остаться точками
/// OSM, иначе сглаживание съедает и без того редкую геометрию длинных улиц.
const MIN_SMOOTH_ANGLE: f32 = 10.0 * PI / 180.0;
/// Доля сегмента, отрезаемая с каждой стороны излома (классический Chaikin).
const CHAIKIN_CUT: f32 = 0.25;

/// Чем закрыт излом ленты дороги.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadJoin {
    /// Статус-кво до сглаживания: свой квад на сегмент, оба конца продлены на
    /// полуширины. Оставлен, чтобы можно было сравнить с прежней картинкой.
    Square,
    /// Сведение по биссектрисе с ограничением длины стыка.
    Miter,
    /// Дуга на внешней стороне излома + полудиск на торце — вид osm-carto.
    #[default]
    Round,
}

impl RoadJoin {
    pub const ALL: [Self; 3] = [Self::Square, Self::Miter, Self::Round];

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Miter => "Miter",
            Self::Round => "Round",
        }
    }
}

/// Сколько раз осевая прогоняется через Chaikin перед построением ленты.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadSmoothing {
    /// Осевая ровно по данным OSM — как в самом OSM, где углы остаются острыми.
    #[default]
    Off,
    Light,
    Strong,
}

impl RoadSmoothing {
    pub const ALL: [Self; 3] = [Self::Off, Self::Light, Self::Strong];

    fn iterations(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Light => 1,
            Self::Strong => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Light => "Light",
            Self::Strong => "Strong",
        }
    }
}

/// Стиль дорожных лент; переключается панелью Roads и BRP, сохраняется в
/// настройках между запусками. Правка пересобирает дорожные слои
/// ([`rebuild_roads`]).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roads")]
pub struct RoadStyle {
    pub join: RoadJoin,
    pub smoothing: RoadSmoothing,
    /// Тёмный кант по краю дороги отдельным слоем под заливкой.
    pub casing: bool,
}

/// Дорожный слой карты — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
pub struct RoadLayerTag;

/// Спавн дорожных слоёв в выбранном стиле. Вызывается из `spawn_map` при входе
/// в мир и из [`rebuild_roads`] при переключении стиля.
pub fn spawn_roads(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    style: RoadStyle,
    roads: &[RoadLine],
    walls: &[WallLine],
) {
    let started = std::time::Instant::now();
    // вершинные цвета — материал один, белый
    let material = materials.add(Color::WHITE);

    let mut alley_casings = MeshBuilder::default();
    let mut alleys = MeshBuilder::default();
    let mut street_casings = MeshBuilder::default();
    let mut streets = MeshBuilder::default();
    let mut wall_ribbons = MeshBuilder::default();

    for road in roads {
        let (casing, fill, casing_color, color) = match road.class {
            RoadClass::Street => (
                &mut street_casings,
                &mut streets,
                ROAD_CASING_COLOR,
                ROAD_COLOR,
            ),
            RoadClass::Alley => (
                &mut alley_casings,
                &mut alleys,
                ALLEY_CASING_COLOR,
                ALLEY_COLOR,
            ),
        };
        let points = centerline(road, style.smoothing);
        if style.casing {
            let width = road.width + 2.0 * casing_width(road.width);
            push_ribbon(casing, &points, width, casing_color.to_linear(), style.join);
        }
        push_ribbon(fill, &points, road.width, color.to_linear(), style.join);
    }

    for wall in walls {
        push_ribbon(
            &mut wall_ribbons,
            &wall.points,
            wall.width,
            WALL_COLOR.to_linear(),
            style.join,
        );
    }

    let vertices = [
        &alley_casings,
        &alleys,
        &street_casings,
        &streets,
        &wall_ribbons,
    ]
    .iter()
    .map(|builder| builder.vertex_count())
    .sum::<usize>();

    for (builder, z, name) in [
        (alley_casings, Z_ALLEY_CASING, "alley_casings"),
        (alleys, Z_ALLEY, "alleys"),
        (street_casings, Z_ROAD_CASING, "road_casings"),
        (streets, Z_ROAD, "roads"),
        (wall_ribbons, Z_WALL, "walls"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            RoadLayerTag,
            Mesh2d(meshes.add(builder.build())),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, z),
            DespawnOnExit(AppState::Playing),
            Name::new(name),
        ));
    }

    info!(
        "road meshing: {vertices} verts in {:?} ({:?}, smoothing {:?}, casing {})",
        started.elapsed(),
        style.join,
        style.smoothing,
        style.casing
    );
}

/// Пересборка дорожных слоёв после переключения стиля из UI или BRP: деспавн
/// старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    style: Res<RoadStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<RoadLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        *style,
        &map.roads,
        &map.walls,
    );
}

/// Толщина канта для дороги такой ширины.
fn casing_width(width: f32) -> f32 {
    (width * CASING_SCALE).clamp(*CASING_RANGE.start(), *CASING_RANGE.end())
}

/// Лента выбранного стиля.
fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    match join {
        RoadJoin::Square => builder.push_polyline(points, width, color),
        RoadJoin::Miter => builder.push_ribbon(
            points,
            false,
            width,
            color,
            RibbonJoin::Miter,
            RibbonCap::Butt,
        ),
        RoadJoin::Round => builder.push_ribbon(
            points,
            false,
            width,
            color,
            RibbonJoin::Round,
            RibbonCap::Round,
        ),
    }
}

/// Осевая, по которой строится лента. Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
fn centerline(road: &RoadLine, smoothing: RoadSmoothing) -> Cow<'_, [Vec2]> {
    let iterations = smoothing.iterations();
    if iterations == 0 || road.passage || road.points.len() < 3 {
        return Cow::Borrowed(&road.points);
    }
    let mut path = road.points.clone();
    for _ in 0..iterations {
        path = chaikin(&path, road.width);
    }
    Cow::Owned(path)
}

/// Срезание углов по Chaikin: излом заменяется парой точек на прилежащих
/// сегментах. Срезаются только изломы круче [`MIN_SMOOTH_ANGLE`], а длина
/// среза зажата шириной дороги — иначе на длинных сегментах осевая уезжает от
/// данных OSM на десятки метров и дорога перестаёт совпадать с домами.
/// Концы пути закреплены.
fn chaikin(points: &[Vec2], width: f32) -> Vec<Vec2> {
    let mut path = Vec::with_capacity(points.len() * 2);
    path.push(points[0]);
    for index in 1..points.len() - 1 {
        let (previous, corner, next) = (points[index - 1], points[index], points[index + 1]);
        let (Some(incoming), Some(outgoing)) = (
            (corner - previous).try_normalize(),
            (next - corner).try_normalize(),
        ) else {
            path.push(corner);
            continue;
        };
        if incoming.angle_to(outgoing).abs() < MIN_SMOOTH_ANGLE {
            path.push(corner);
            continue;
        }
        let back = (corner.distance(previous) * CHAIKIN_CUT).min(width);
        let forward = (next.distance(corner) * CHAIKIN_CUT).min(width);
        path.push(corner - incoming * back);
        path.push(corner + outgoing * forward);
    }
    path.push(points[points.len() - 1]);
    path
}

#[cfg(test)]
mod tests;
