//! Портал: анимированный спрайт на опушке парка, присутствует с начала сцены.

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};
use crate::settings::{PORTAL_DIAMETER, PORTAL_POS, Z_PORTAL};

/// Кадры спрайтшита: 3 × 3 сетка по 160 px.
const FRAME_SIZE: u32 = 160;
const FRAME_COLS: u32 = 3;
const FRAME_ROWS: u32 = 3;
const FRAME_COUNT: usize = 9;
const FRAME_SECS: f32 = 0.12;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Portal;

#[derive(Component)]
pub struct PortalAnimation(Timer);

/// Фактическая позиция портала. Стартует с хинта `PORTAL_POS`; после
/// заполнения navmesh снапится к ближайшему проходимому тайлу.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct PortalPos(pub Vec2);

impl Default for PortalPos {
    fn default() -> Self {
        Self(PORTAL_POS)
    }
}

pub struct PortalPlugin;

impl Plugin for PortalPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Portal>()
            .register_type::<PortalPos>()
            .init_resource::<PortalPos>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_portal.in_set(WorldInitSet::Spawn),
            )
            .add_systems(Update, animate_portal);
    }
}

fn spawn_portal(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    portal_pos: Res<PortalPos>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(FRAME_SIZE),
        FRAME_COLS,
        FRAME_ROWS,
        None,
        None,
    ));

    let mut sprite = Sprite::from_atlas_image(
        asset_server.load("portal_spritesheet.png"),
        TextureAtlas { layout, index: 0 },
    );
    sprite.custom_size = Some(Vec2::splat(PORTAL_DIAMETER));

    commands.spawn((
        sprite,
        Transform::from_translation(portal_pos.0.extend(Z_PORTAL)),
        Portal,
        PortalAnimation(Timer::from_seconds(FRAME_SECS, TimerMode::Repeating)),
        Name::new("portal"),
    ));
}

fn animate_portal(time: Res<Time>, mut query: Query<(&mut Sprite, &mut PortalAnimation)>) {
    for (mut sprite, mut animation) in &mut query {
        animation.0.tick(time.delta());
        if !animation.0.just_finished() {
            continue;
        }
        if let Some(atlas) = sprite.texture_atlas.as_mut() {
            atlas.index = (atlas.index + 1) % FRAME_COUNT;
        }
    }
}
