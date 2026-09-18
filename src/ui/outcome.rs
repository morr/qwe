//! Плашка исхода посреди экрана: победа или поражение, часы симуляции, души и
//! подсказка «R — restart». Спавнится в `Startup` скрытой и показывается
//! только по `Outcome`, а не по `GameUiRoot`: та группа — «мир идёт», эта —
//! «мир кончился». Только ASCII: `default_font` без кириллицы.

use bevy::feathers::theme::ThemeTextColor;
use bevy::feathers::tokens;
use bevy::prelude::*;

use crate::outcome::{LossReason, Outcome};
use crate::sim_time::SimClock;
use crate::souls::Souls;
use crate::ui::{panel_background, panel_title};

#[derive(Component)]
struct OutcomePlaque;

#[derive(Component)]
struct OutcomeTitle;

#[derive(Component)]
struct OutcomeDetails;

pub struct UiOutcomePlugin;

impl Plugin for UiOutcomePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_outcome_plaque).add_systems(
            Update,
            sync_outcome_plaque.run_if(resource_changed::<Outcome>),
        );
    }
}

fn render_outcome_plaque(mut commands: Commands) {
    commands.spawn((
        crate::ui::ui_node(Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(40),
            // центр плашки — в точке, а не её левый верхний угол
            margin: UiRect {
                left: px(-160.),
                ..default()
            },
            width: px(320.),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(8.),
            padding: UiRect::all(px(16.)),
            ..default()
        }),
        panel_background(),
        Visibility::Hidden,
        OutcomePlaque,
        Name::new("outcome_plaque"),
        children![
            (panel_title("outcome"), OutcomeTitle),
            (
                Text::default(),
                ThemeTextColor(tokens::TEXT_DIM),
                TextLayout::justify(Justify::Center),
                OutcomeDetails,
            ),
        ],
    ));
}

/// Показать или спрятать плашку по исходу; текст — на переходе, а не каждый
/// кадр: мир на паузе, числа больше не меняются.
fn sync_outcome_plaque(
    outcome: Res<Outcome>,
    clock: Res<SimClock>,
    souls: Res<Souls>,
    mut plaque: Single<&mut Visibility, With<OutcomePlaque>>,
    mut title: Single<&mut Text, (With<OutcomeTitle>, Without<OutcomeDetails>)>,
    mut details: Single<&mut Text, (With<OutcomeDetails>, Without<OutcomeTitle>)>,
) {
    let (heading, verdict) = match *outcome {
        Outcome::Running => {
            **plaque = Visibility::Hidden;
            return;
        }
        Outcome::Won { .. } => ("VICTORY", "the heart is corrupted"),
        Outcome::Lost {
            reason: LossReason::Stalemate,
            ..
        } => ("DEFEAT", "no demons left and no souls to summon"),
    };
    title.0 = heading.to_string();
    details.0 = format!(
        "{verdict}\nT+{}  souls {} / {}\nR - restart",
        clock.elapsed.max(0.0) as u64,
        souls.available(),
        souls.earned
    );
    **plaque = Visibility::Inherited;
}
