//! Ручки толпы — две группы нижней половины панели: **Separation**
//! (`movement/separation/`) и **Slots** (`movement/destination.rs`). Сами
//! ручки — общий кит (`ui::knob`), здесь только их состав, раскладка по
//! группам и то, чего у остальных панелей нет: строки расталкивания прячутся
//! и гаснут вслед за режимом мира.

use bevy::prelude::*;

use super::{NavPanelValues, display_of, indent_slider_row};
use crate::determinism::Determinism;
use crate::human::HumanStyle;
use crate::movement::{SeparationLab, SlotLab, SlotSearch, separation_allowed_by_mode};
use crate::navigation::PolymeshDebug;
use crate::settings::{
    CLAIM_SEARCH_MAX, CLAIM_SEARCH_MIN, CLAIM_SEARCH_STEP, HUMAN_BODY_RADIUS_MAX,
    HUMAN_BODY_RADIUS_MIN, HUMAN_BODY_RADIUS_STEP, SEPARATION_LEFT_SHARE_MAX,
    SEPARATION_LEFT_SHARE_MIN, SEPARATION_LEFT_SHARE_STEP, SEPARATION_PASS_SQUEEZE_MAX,
    SEPARATION_PASS_SQUEEZE_MIN, SEPARATION_PASS_SQUEEZE_STEP, SLOT_REGROUP_MAX, SLOT_REGROUP_MIN,
    SLOT_REGROUP_STEP,
};
use crate::ui::knob::{SliderBinding, spawn_knob};
use crate::ui::rows::RowInert;

/// Группа ручек толпы — только для того, чтобы разложить их по заголовкам.
/// Ни ресурса, ни компонента: прятать группы незачем (см. док модуля).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum KnobGroup {
    Separation,
    Slots,
}

/// Строка-тумблер расталкивания: собственная ветка в подсветке, потому что под
/// детерминизмом и на сеточной навигации она не откликается вовсе.
#[derive(Component)]
pub(super) struct SeparationToggleRow;

/// Ползунок группы Separation — прячется, когда расталкивание не работает.
#[derive(Component)]
pub(super) struct SeparationKnobRow;

/// Ползунки одной группы. Каждая ручка правит **свой** ресурс — «Body radius»
/// живёт в `HumanStyle`, соседний «Slot search» в `SlotSearch`, — и привязка
/// на строке разводит их поодиночке; общий на группу enum держал бы все
/// четыре ресурса одним `SystemParam` ради каждой протяжки.
pub(super) fn spawn_knob_rows(
    commands: &mut Commands,
    panel: Entity,
    values: &NavPanelValues,
    group: KnobGroup,
) {
    let rows = match group {
        KnobGroup::Separation => vec![
            spawn_knob(
                commands,
                panel,
                "Pass squeeze",
                values.separation_lab(),
                SliderBinding::<SeparationLab> {
                    get: |lab| lab.pass_squeeze,
                    set: |lab, value| lab.pass_squeeze = value,
                    range: (
                        SEPARATION_PASS_SQUEEZE_MIN,
                        SEPARATION_PASS_SQUEEZE_MAX,
                        SEPARATION_PASS_SQUEEZE_STEP,
                    ),
                    text: |value| format!("{value:.2}"),
                },
            ),
            spawn_knob(
                commands,
                panel,
                "Left share",
                values.separation_lab(),
                SliderBinding::<SeparationLab> {
                    get: |lab| lab.left_share,
                    set: |lab, value| lab.left_share = value,
                    range: (
                        SEPARATION_LEFT_SHARE_MIN,
                        SEPARATION_LEFT_SHARE_MAX,
                        SEPARATION_LEFT_SHARE_STEP,
                    ),
                    text: |value| format!("{value:.2}"),
                },
            ),
        ],
        KnobGroup::Slots => vec![
            spawn_knob(
                commands,
                panel,
                "Body radius",
                values.human(),
                SliderBinding::<HumanStyle> {
                    get: |human| human.body_radius,
                    set: |human, value| human.body_radius = value,
                    range: (
                        HUMAN_BODY_RADIUS_MIN,
                        HUMAN_BODY_RADIUS_MAX,
                        HUMAN_BODY_RADIUS_STEP,
                    ),
                    text: |value| format!("{value:.2} m"),
                },
            ),
            spawn_knob(
                commands,
                panel,
                "Slot search",
                values.search(),
                SliderBinding::<SlotSearch> {
                    get: |search| search.0,
                    set: |search, value| search.0 = value,
                    range: (CLAIM_SEARCH_MIN, CLAIM_SEARCH_MAX, CLAIM_SEARCH_STEP),
                    text: |value| format!("{value:.0} m"),
                },
            ),
            spawn_knob(
                commands,
                panel,
                "Regroup",
                values.slot_lab(),
                SliderBinding::<SlotLab> {
                    get: |slots| slots.regroup,
                    set: |slots, value| slots.regroup = value,
                    range: (SLOT_REGROUP_MIN, SLOT_REGROUP_MAX, SLOT_REGROUP_STEP),
                    text: |value| format!("{value:.2} m"),
                },
            ),
        ],
    };

    for row in rows {
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

/// Единственная строка панелей, которая бывает неотзывчивой: под детерминизмом
/// и на сеточной навигации расталкивания нет вовсе, и тумблер молча ничего не
/// переключает. [`RowInert`] снимает с неё подсветку — обещать реакцию на
/// курсор там, где клик ничего не сделает, хуже, чем не подсвечивать.
///
/// Метка, а не проверка внутри общей подсветки: та крутится по строкам всех
/// панелей и про режимы мира знать не должна. Начальное состояние ставит
/// [`render_navigation_panel`](super::render_navigation_panel) — эта система
/// ходит по `resource_changed`, а на первом кадре ни один ресурс ещё не
/// «менялся».
pub(super) fn sync_separation_row_inert(
    determinism: Res<Determinism>,
    polymesh: Res<PolymeshDebug>,
    rows: Query<Entity, With<SeparationToggleRow>>,
    mut commands: Commands,
) {
    let allowed = separation_allowed_by_mode(determinism.0, polymesh.enabled);
    for row in &rows {
        if allowed {
            commands.entity(row).remove::<RowInert>();
        } else {
            commands.entity(row).insert(RowInert);
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
pub(super) fn sync_separation_knob_visibility(
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
