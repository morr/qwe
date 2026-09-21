//! Метка `BRP` в левом верхнем углу: это окно запустил агент.
//!
//! Пользователь держит своё окно на дефолтном порту, агент запускается с
//! `BRP_PORT=…` в окружении (см. `.claude/live-app-project.md`) — по нему одно
//! окно и отличается от другого. В заголовке окна порт уже стоит, но заголовок
//! не виден ни на скриншоте, ни в полноэкранном окне, а перепутать два
//! одинаковых города легко.
//!
//! Витрины `examples/demos/*` BRP обычно не поднимают, и порта у них нет, —
//! они поднимают `AgentBadgePlugin`, который узнаёт агента по `CLAUDECODE`:
//! эту переменную Claude Code ставит каждому процессу своей сессии.

use bevy::picking::Pickable;
use bevy::prelude::*;

use super::{UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW};

/// Стоит, только когда окно запустил агент: в игре — по `BRP_PORT` (ставит
/// `main.rs`), в витрине — по `AgentBadgePlugin`. У обычного запуска ресурса
/// нет — метка тогда не спавнится вовсе.
#[derive(Resource)]
pub struct AgentBrpSession;

/// Сама метка — по ней `offset_below_brp_badge` отмеряет, на сколько съехать
/// вниз тому, что стоит в её углу.
#[derive(Component)]
pub struct BrpBadge;

/// Узел в левом верхнем углу, который уступает метке место и съезжает под неё:
/// левая колонка игры (`ui/shell.rs`), плашка витрины.
#[derive(Component)]
pub struct BelowBrpBadge;

/// Светло-красный полупрозрачный фон: заметно поверх любой карты, но не глушит
/// её и не читается как часть игрового UI (тот тёмный, см. `ui_color`).
const BADGE_COLOR: Color = Color::srgba(0.85, 0.24, 0.22, 0.85);

pub struct UiBrpBadgePlugin;

impl Plugin for UiBrpBadgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            render_brp_badge.run_if(resource_exists::<AgentBrpSession>),
        )
        .add_systems(
            Update,
            // метка стоит только в агентских запусках, и только тогда углу
            // есть что обходить
            offset_below_brp_badge.run_if(resource_exists::<AgentBrpSession>),
        );
    }
}

/// Метка для витрин: игра поднимает `UiBrpBadgePlugin` через `UiPlugin`, а
/// витрина — этот плагин рядом с `QuitOnEscPlugin`. Агент узнаётся по
/// `CLAUDECODE`, а `BRP_PORT` — на случай витрины, поднятой с портом по
/// рецепту игры (`crowd_demo`).
pub struct AgentBadgePlugin;

impl Plugin for AgentBadgePlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("CLAUDECODE").is_some() || std::env::var_os("BRP_PORT").is_some() {
            app.insert_resource(AgentBrpSession);
        }
        app.add_plugins(UiBrpBadgePlugin);
    }
}

/// Метка живёт весь запуск, включая экран загрузки: не `GameUiRoot` — прятать
/// её вместе с панелями незачем, окна путаются как раз пока карта грузится.
fn render_brp_badge(mut commands: Commands) {
    commands.spawn((
        BrpBadge,
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

/// Метка занимает левый верхний угол, но только в агентских запусках — то, что
/// стоит в нём, уступает ей место и съезжает под неё.
///
/// Высота читается из `ComputedNode`, то есть с прошлого кадра, и она в
/// **физических** пикселях, тогда как `Node::top` — в логических: без
/// `inverse_scale_factor` на retina зазор удваивается. `top` пишется только
/// когда реально изменился —
/// `Node` не `set_if_neq`-компонент, и безусловная запись метила бы его
/// изменённым каждый кадр, заставляя `bevy_ui` пересчитывать раскладку зря.
fn offset_below_brp_badge(
    badge: Single<&ComputedNode, With<BrpBadge>>,
    mut below: Query<&mut Node, With<BelowBrpBadge>>,
) {
    let top = px(UI_SCREEN_EDGE_PX_OFFSET * 2.0 + badge.size.y * badge.inverse_scale_factor);
    for mut node in &mut below {
        if node.top != top {
            node.top = top;
        }
    }
}
