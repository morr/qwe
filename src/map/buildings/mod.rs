//! Слои зданий и режимы отображения их высоты из OSM. Пять режимов,
//! переключаемых на лету (панель Buildings, `ui/buildings.rs`): фасадная
//! полоса (статус-кво), длинные тени как у деревьев, тени с тонировкой крыш
//! по высоте, 2.5D-экструзия в стиле watabou и всё разом. Правка
//! `BuildingHeightMode` пересобирает только зданиевые слои
//! (`rebuild_buildings`).
//!
//! Геометрия разнесена по двум подмодулям: [`arches`] режет проходы
//! `building_passage` сквозь стены, [`layers`] собирает сами меши слоёв.

mod arches;
mod layers;

use std::ops::RangeInclusive;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use self::layers::{extrusion_builder, facade_and_roof_builders, shadow_builder};
use crate::loading::AppState;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{AreaKind, MapData, PolyArea, RoadLine};
use crate::settings::Z_BUILDING;

const ROOF_COLOR: Color = Color::srgb(0.949, 0.929, 0.878);
const FACADE_COLOR: Color = Color::srgb(0.663, 0.616, 0.529);
const KREMLIN_ROOF_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);
const KREMLIN_FACADE_COLOR: Color = Color::srgb(0.42, 0.18, 0.15);

/// Высота здания без OSM-данных — пятиэтажка. Через `FACADE_SCALE` даёт
/// прежние 3 м фасадной полосы, так что режим Facade без высот не меняется.
const DEFAULT_BUILDING_HEIGHT: f32 = 15.0;
/// Фасады чуть ниже крыш: крыша соседа сверху прикрывает полосу — иначе
/// широкая полоса высотки залезала бы на низкого соседа.
const Z_FACADE: f32 = Z_BUILDING - 0.1;

/// Направление тени — то же, что у крон деревьев (`trees::SHADOW_DIR`):
/// 30° вниз-вправо, нормировано. Один источник света на всю карту.
const SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);
/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`, как у крон.
const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);
/// Тени зданий — под всеми зданиевыми слоями (фасады 4.9, крыши и экструзия
/// 5.0): крыша или стена соседа сама маскирует тень, и тень никогда не
/// ложится на крышу дома той же высоты — дешёвая замена честному учёту
/// высот. Выше портала (4) и трупов (3): они на улице и в тени по смыслу.
const Z_BUILDING_SHADOW: f32 = Z_BUILDING - 0.5;

/// Метров подъёма крыши на метр высоты в 2.5D: драматичнее фасадной полосы,
/// но карта остаётся видом сверху, а не изометрией.
const EXTRUDE_SCALE: f32 = 0.35;
/// Границы подъёма крыши, м.
const EXTRUDE_RANGE: RangeInclusive<f32> = 2.5..=30.0;

/// Режим отображения высоты зданий; переключается панелью Buildings и BRP,
/// сохраняется в настройках между запусками.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "buildings", key = "height_mode")]
pub enum BuildingHeightMode {
    /// Статус-кво: тёмная полоса фасада, сдвинутая вниз на долю высоты.
    #[default]
    Facade,
    /// Полоса фасада + длинная тень, длина пропорциональна высоте.
    Shadows,
    /// Тени + тонировка крыш по высоте: выше — темнее и глуше.
    ShadowsTint,
    /// 2.5D: крыша поднята на долю высоты, между контуром и крышей — стены.
    Extrusion,
    /// Всё разом: 2.5D-экструзия + тонировка крыш + длинные тени.
    ExtrusionShadowsTint,
}

impl BuildingHeightMode {
    pub const ALL: [Self; 5] = [
        Self::Facade,
        Self::Shadows,
        Self::ShadowsTint,
        Self::Extrusion,
        Self::ExtrusionShadowsTint,
    ];

    /// Следующий по циклу — для кнопки-переключателя.
    pub fn next(self) -> Self {
        match self {
            Self::Facade => Self::Shadows,
            Self::Shadows => Self::ShadowsTint,
            Self::ShadowsTint => Self::Extrusion,
            Self::Extrusion => Self::ExtrusionShadowsTint,
            Self::ExtrusionShadowsTint => Self::Facade,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Facade => "Facade",
            Self::Shadows => "Shadows",
            Self::ShadowsTint => "Shadows+tint",
            Self::Extrusion => "2.5D",
            Self::ExtrusionShadowsTint => "2.5D+shadows+tint",
        }
    }
}

/// Зданиевый слой карты — чтобы пересборка режима знала, что деспавнить.
#[derive(Component)]
pub struct BuildingLayerTag;

/// Спавн зданиевых слоёв в выбранном режиме. Вызывается из `spawn_map` при
/// входе в мир и из `rebuild_buildings` при переключении режима.
pub fn spawn_buildings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    mode: BuildingHeightMode,
    buildings: &[PolyArea],
    passages: &[RoadLine],
) {
    // вершинные цвета — материал один, белый; тени — свой blend-материал
    let opaque = materials.add(Color::WHITE);
    let mut skipped = 0;
    let mut spawn_layer =
        |commands: &mut Commands, meshes: &mut Assets<Mesh>, builder: MeshBuilder, z, name| {
            skipped += builder.skipped_polygons();
            if builder.is_empty() {
                return;
            }
            commands.spawn((
                BuildingLayerTag,
                Mesh2d(meshes.add(builder.build())),
                MeshMaterial2d(opaque.clone()),
                Transform::from_xyz(0.0, 0.0, z),
                DespawnOnExit(AppState::Playing),
                Name::new(name),
            ));
        };

    match mode {
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint => {
            let tinted = mode == BuildingHeightMode::ExtrusionShadowsTint;
            spawn_layer(
                commands,
                meshes,
                extrusion_builder(buildings, passages, tinted),
                Z_BUILDING,
                "building_extruded",
            );
        }
        BuildingHeightMode::Facade
        | BuildingHeightMode::Shadows
        | BuildingHeightMode::ShadowsTint => {
            let tinted = mode == BuildingHeightMode::ShadowsTint;
            let (facades, roofs) = facade_and_roof_builders(buildings, passages, tinted);
            spawn_layer(commands, meshes, facades, Z_FACADE, "building_facades");
            spawn_layer(commands, meshes, roofs, Z_BUILDING, "building_roofs");
        }
    }

    if matches!(
        mode,
        BuildingHeightMode::Shadows
            | BuildingHeightMode::ShadowsTint
            | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        let shadows = shadow_builder(
            buildings,
            passages,
            mode == BuildingHeightMode::ExtrusionShadowsTint,
        );
        if !shadows.is_empty() {
            commands.spawn((
                BuildingLayerTag,
                Mesh2d(meshes.add(shadows.build())),
                MeshMaterial2d(materials.add(ColorMaterial {
                    alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.0, Z_BUILDING_SHADOW),
                DespawnOnExit(AppState::Playing),
                Name::new("building_shadows"),
            ));
        }
    }

    if skipped > 0 {
        warn!("building meshing: {skipped} degenerate polygons skipped");
    }
}

/// Пересборка зданиевых слоёв после переключения режима из UI или BRP:
/// деспавн старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_buildings(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mode: Res<BuildingHeightMode>,
    map: Res<MapData>,
    existing: Query<Entity, With<BuildingLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        *mode,
        &map.buildings,
        &map.roads,
    );
}

/// Высота здания с дефолтом — `None` в OSM это норма, а не ошибка.
fn height_or_default(building: &PolyArea) -> f32 {
    building.height.unwrap_or(DEFAULT_BUILDING_HEIGHT)
}

/// На сколько в этом режиме поднята крыша относительно настоящего контура.
/// В режимах без экструзии — ноль.
///
/// Публично, потому что оверлей дверей обязан повторять тот же сдвиг: дверь
/// живёт на настоящем контуре, а он в 2.5D уходит под нарисованный дом, и
/// метка на северной грани иначе читается как «дверь в середине здания».
pub fn extrusion_lift(building: &PolyArea, mode: BuildingHeightMode) -> Vec2 {
    if !matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        return Vec2::ZERO;
    }
    let height = (height_or_default(building) * EXTRUDE_SCALE)
        .clamp(*EXTRUDE_RANGE.start(), *EXTRUDE_RANGE.end());
    Vec2::new(0.0, height)
}

/// Базовые цвета крыши и фасада по типу здания.
fn base_colors(building: &PolyArea) -> (Color, Color) {
    match building.kind {
        AreaKind::Kremlin => (KREMLIN_ROOF_COLOR, KREMLIN_FACADE_COLOR),
        _ => (ROOF_COLOR, FACADE_COLOR),
    }
}

#[cfg(test)]
mod tests;
