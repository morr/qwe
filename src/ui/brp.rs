//! Метка `BRP` в левом верхнем углу: это окно запустил агент.
//!
//! Пользователь держит своё окно на дефолтном порту, агент запускается с
//! `BRP_PORT=…` в окружении (см. `.claude/live-app-project.md`) — по нему одно
//! окно и отличается от другого. В заголовке окна порт уже стоит, но заголовок
//! не виден ни на скриншоте, ни в полноэкранном окне, а перепутать два
//! одинаковых города легко.

use bevy::picking::Pickable;
use bevy::prelude::*;

use super::{UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW};

/// Стоит, только когда порт BRP задан снаружи через `BRP_PORT`. У обычного
/// запуска ресурса нет — метка тогда не спавнится вовсе.
#[derive(Resource)]
pub struct AgentBrpSession;

/// Светло-красный полупрозрачный фон: заметно поверх любой карты, но не глушит
/// её и не читается как часть игрового UI (тот тёмный, см. `ui_color`).
const BADGE_COLOR: Color = Color::srgba(0.85, 0.24, 0.22, 0.85);

pub struct UiBrpBadgePlugin;

impl Plugin for UiBrpBadgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            render_brp_badge.run_if(resource_exists::<AgentBrpSession>),
        );
    }
}

/// Метка живёт весь запуск, включая экран загрузки: не `GameUiRoot` — прятать
/// её вместе с панелями незачем, окна путаются как раз пока карта грузится.
fn render_brp_badge(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(UI_SCREEN_EDGE_PX_OFFSET),
            left: px(UI_SCREEN_EDGE_PX_OFFSET),
            padding: UiRect {
                top: px(8.),
                right: px(10.),
                bottom: px(8.),
                left: px(10.),
            },
            ..default()
        },
        BackgroundColor(BADGE_COLOR),
        // ничего не нажимается: без этого метка попадала бы в `HoverMap` и
        // гасила протяжку камеры в своём углу (см. `camera::pointer_over_ui`)
        Pickable::IGNORE,
        Name::new("brp_badge"),
        children![(
            Text::new("BRP"),
            TextFont {
                font_size: FontSize::Px(16.),
                ..default()
            },
            TextColor(Color::WHITE),
            UI_TEXT_SHADOW,
            Pickable::IGNORE,
        )],
    ));
}
