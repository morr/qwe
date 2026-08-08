//! Ручки толпы — две группы нижней половины панели: **Separation**
//! (`movement/separation/`) и **Slots** (`movement/destination.rs`). Одним
//! enum'ом на обе, а не маркером на строку: спавн, синхронизация подписи и
//! синхронизация бегунка — по разу на все, и новая ручка добавляется веткой.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui_widgets::{SliderValue, ValueChange};

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
use crate::ui::rows::RowInert;
use crate::ui::slider::{SliderRow, apply_step, retarget, spawn_slider_row};

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

/// Числовая ручка толпы. Одним enum'ом на обе подвкладки, а не маркером на
/// строку: спавн, синхронизация подписи и синхронизация бегунка — по разу на
/// все пять, и новая ручка добавляется одной веткой.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum Knob {
    PassSqueeze,
    LeftShare,
    BodyRadius,
    SlotSearch,
    Regroup,
}

/// Подпись значения такой строки.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) struct KnobValueLabel(Knob);

/// Её ползунок.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) struct KnobSlider(Knob);

impl Knob {
    const ALL: [Self; 5] = [
        Self::PassSqueeze,
        Self::LeftShare,
        Self::BodyRadius,
        Self::SlotSearch,
        Self::Regroup,
    ];

    fn group(self) -> KnobGroup {
        match self {
            Self::PassSqueeze | Self::LeftShare => KnobGroup::Separation,
            Self::BodyRadius | Self::SlotSearch | Self::Regroup => KnobGroup::Slots,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PassSqueeze => "Pass squeeze",
            Self::LeftShare => "Left share",
            Self::BodyRadius => "Body radius",
            Self::SlotSearch => "Slot search",
            Self::Regroup => "Regroup",
        }
    }

    /// `(min, max, шаг)` — из `settings.rs`, как у остальных ползунков.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::PassSqueeze => (
                SEPARATION_PASS_SQUEEZE_MIN,
                SEPARATION_PASS_SQUEEZE_MAX,
                SEPARATION_PASS_SQUEEZE_STEP,
            ),
            Self::LeftShare => (
                SEPARATION_LEFT_SHARE_MIN,
                SEPARATION_LEFT_SHARE_MAX,
                SEPARATION_LEFT_SHARE_STEP,
            ),
            Self::BodyRadius => (
                HUMAN_BODY_RADIUS_MIN,
                HUMAN_BODY_RADIUS_MAX,
                HUMAN_BODY_RADIUS_STEP,
            ),
            Self::SlotSearch => (CLAIM_SEARCH_MIN, CLAIM_SEARCH_MAX, CLAIM_SEARCH_STEP),
            Self::Regroup => (SLOT_REGROUP_MIN, SLOT_REGROUP_MAX, SLOT_REGROUP_STEP),
        }
    }

    /// Значение ручки. По ссылкам на сами ресурсы, а не по `NavPanelValues`:
    /// то же чтение нужно наблюдателю строки, а у него они `ResMut`.
    pub(super) fn get(
        self,
        lab: &SeparationLab,
        slots: &SlotLab,
        human: &HumanStyle,
        search: &SlotSearch,
    ) -> f32 {
        match self {
            Self::PassSqueeze => lab.pass_squeeze,
            Self::LeftShare => lab.left_share,
            Self::BodyRadius => human.body_radius,
            Self::SlotSearch => search.0,
            Self::Regroup => slots.regroup,
        }
    }

    fn set(self, knobs: &mut KnobResources, value: f32) {
        match self {
            Self::PassSqueeze => knobs.separation_lab.pass_squeeze = value,
            Self::LeftShare => knobs.separation_lab.left_share = value,
            Self::BodyRadius => knobs.human.body_radius = value,
            Self::SlotSearch => knobs.search.0 = value,
            Self::Regroup => knobs.slot_lab.regroup = value,
        }
    }

    /// Единица измерения в подписи: у радиусов и возврата это метры, у двух
    /// долей — голое число.
    fn value_text(self, value: f32) -> String {
        match self {
            Self::PassSqueeze | Self::LeftShare => format!("{value:.2}"),
            Self::BodyRadius | Self::Regroup => format!("{value:.2} m"),
            Self::SlotSearch => format!("{value:.0} m"),
        }
    }
}

/// Ресурсы, которые ползунки толпы правят. Отдельным `SystemParam`, чтобы
/// наблюдатель строки брал их одним аргументом, а не четырьмя.
#[derive(SystemParam)]
struct KnobResources<'w> {
    separation_lab: ResMut<'w, SeparationLab>,
    slot_lab: ResMut<'w, SlotLab>,
    human: ResMut<'w, HumanStyle>,
    search: ResMut<'w, SlotSearch>,
}

impl KnobResources<'_> {
    fn value(&self, knob: Knob) -> f32 {
        knob.get(
            &self.separation_lab,
            &self.slot_lab,
            &self.human,
            &self.search,
        )
    }
}

/// Ползунки одной группы, в порядке [`Knob::ALL`].
pub(super) fn spawn_knob_rows(
    commands: &mut Commands,
    panel: Entity,
    values: &NavPanelValues,
    group: KnobGroup,
) {
    for knob in Knob::ALL.into_iter().filter(|knob| knob.group() == group) {
        let value = values.knob(knob);
        let row = spawn_slider_row(
            commands,
            panel,
            SliderRow {
                label: knob.label(),
                value,
                value_text: knob.value_text(value),
                range: knob.range(),
            },
            KnobValueLabel(knob),
            KnobSlider(knob),
            move |change: On<ValueChange<f32>>,
                  mut commands: Commands,
                  mut knobs: KnobResources| {
                let (min, max, step) = knob.range();
                let stepped = apply_step(&change, &mut commands, (min, max, step));
                // ресурс правится только на реальной смене шага: иначе каждый
                // пиксель протяжки метил бы его изменённым
                if (knobs.value(knob) - stepped).abs() > f32::EPSILON {
                    knob.set(&mut knobs, stepped);
                }
            },
        );
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
/// [`render_navigation_panel`] — эта система ходит по `resource_changed`, а на
/// первом кадре ни один ресурс ещё не «менялся».
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

/// Подписи и бегунки ручек толпы вслед за ресурсами: их правят не только эти
/// ползунки (BRP, панель демо-сцены, пресеты стенда), а расходиться показанному
/// и настоящему нельзя.
pub(super) fn sync_knob_values(
    values: NavPanelValues,
    mut commands: Commands,
    mut labels: Query<(&KnobValueLabel, &mut Text)>,
    sliders: Query<(Entity, &KnobSlider, &SliderValue)>,
) {
    for (label, mut text) in &mut labels {
        let next = label.0.value_text(values.knob(label.0));
        if text.0 != next {
            text.0 = next;
        }
    }
    for (entity, slider, value) in &sliders {
        let next = values.knob(slider.0);
        retarget(&mut commands, entity, value.0, next);
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
