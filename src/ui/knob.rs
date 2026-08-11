//! Ручка панели — строка, привязанная к полю ресурса.
//!
//! Панелей стиля шесть, и путь новой ручки был один и тот же в каждой:
//! наблюдатель протяжки (округлить шаг, сравнить с текущим, записать поле) и
//! система, тянущая подпись с бегунком за ресурсом, когда его правят мимо
//! панели — по BRP, из сохранённых настроек, из пресета стенда. Наблюдателей
//! таких набралось тринадцать, систем — восемь, и все различались только тем,
//! какое поле какого ресурса читают и пишут.
//!
//! Здесь это различие — данные: [`SliderBinding`] держит четвёрку функций
//! (взять, положить, диапазон, текст), а наблюдатель и система заведены по
//! разу на **ресурс** ([`AddKnobsExt::add_knobs`]), а не на ручку.
//!
//! Функции, а не enum на панель (как было в `ui/navigation/knobs.rs`): ручки
//! одной панели ходят в разные ресурсы — «Body radius» правит `HumanStyle`,
//! соседний «Slot search» — `SlotSearch`, — и enum'у на панель приходилось
//! тащить их все одним `SystemParam`. Привязка на строке разводит ручки по их
//! ресурсам поодиночке.

use bevy::ecs::component::Mutable;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, SliderValue, ValueChange};

use crate::ui::rows::spawn_value_row;
use crate::ui::slider::{SliderRow, apply_step, retarget, spawn_slider_row};

/// Ресурс, у которого бывают ручки. Псевдоним одного длинного набора границ:
/// `ResMut` требует изменяемости на уровне типа (в 0.19 ресурс — компонент, а
/// у компонента есть `Mutability`), и повторять это в каждой сигнатуре кита
/// незачем.
pub trait Knobbed: Resource<Mutability = Mutable> {}

impl<R: Resource<Mutability = Mutable>> Knobbed for R {}

/// Привязка строки-ползунка к полю ресурса `R`.
///
/// Компонент строки и ползунка разом: наблюдателю нужна привязка на ползунке,
/// синхронизации — на обоих (подпись и бегунок — разные сущности).
#[derive(Component)]
pub struct SliderBinding<R: Knobbed> {
    pub get: fn(&R) -> f32,
    pub set: fn(&mut R, f32),
    /// `(min, max, шаг)` — из `settings.rs`, как у всех ползунков.
    pub range: (f32, f32, f32),
    /// Как значение показывается в подписи: единицы, знаки, проценты.
    pub text: fn(f32) -> String,
}

// `derive` вывел бы `R: Copy`, а `R` — ресурс, и копируемым он не бывает.
// Сама привязка — четыре указателя на функции и тройка чисел, копируется даром.
impl<R: Knobbed> Clone for SliderBinding<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Knobbed> Copy for SliderBinding<R> {}

impl<R: Knobbed> SliderBinding<R> {
    fn value_text(&self, value: f32) -> String {
        (self.text)(value)
    }
}

/// Подпись значения такой строки — своим компонентом, потому что живёт на
/// отдельной сущности от ползунка.
#[derive(Component)]
pub struct KnobValueLabel<R: Knobbed>(pub SliderBinding<R>);

/// Строка-ползунок, привязанная к полю ресурса. Возвращает блок строки —
/// панель довешивает на него свои метки видимости.
pub fn spawn_knob<R: Knobbed>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    resource: &R,
    binding: SliderBinding<R>,
) -> Entity {
    let value = (binding.get)(resource);
    spawn_slider_row(
        commands,
        panel,
        SliderRow {
            label,
            value,
            value_text: binding.value_text(value),
            range: binding.range,
        },
        KnobValueLabel(binding),
        binding,
        on_knob_dragged::<R>,
    )
}

/// Протяжка ползунка: округлить до шага и записать поле — но только на
/// **реальной** смене шага, иначе каждый пиксель протяжки метил бы ресурс
/// изменённым, а за меткой стоят пересборка мешей и запись настроек.
fn on_knob_dragged<R: Knobbed>(
    change: On<ValueChange<f32>>,
    bindings: Query<&SliderBinding<R>>,
    mut commands: Commands,
    mut resource: ResMut<R>,
) {
    let Ok(binding) = bindings.get(change.source) else {
        return;
    };
    let binding = *binding;
    let stepped = apply_step(&change, &mut commands, binding.range);
    if ((binding.get)(&resource) - stepped).abs() > f32::EPSILON {
        (binding.set)(&mut resource, stepped);
    }
}

/// Подписи и бегунки вслед за ресурсом: его правят не только эти ползунки
/// (BRP, сохранённые настройки, пресеты стенда), а расходиться показанному и
/// настоящему нельзя. `SliderValue` — immutable-компонент, меняется только
/// вставкой, поэтому [`retarget`], а не присваивание.
fn sync_knob_values<R: Knobbed>(
    resource: Res<R>,
    mut labels: Query<(&KnobValueLabel<R>, &mut Text)>,
    sliders: Query<(Entity, &SliderBinding<R>, &SliderValue)>,
    mut commands: Commands,
) {
    for (label, mut text) in &mut labels {
        let next = label.0.value_text((label.0.get)(&resource));
        if text.0 != next {
            text.0 = next;
        }
    }
    for (entity, binding, value) in &sliders {
        retarget(&mut commands, entity, value.0, (binding.get)(&resource));
    }
}

/// Привязка строки-**кнопки** к полю ресурса: полей ввода в `bevy_ui` нет,
/// поэтому нечисловая ручка — кнопка, листающая значение по кругу.
///
/// `cycle` листает сам (а не «следующий из списка `ALL`»): по кругу ходят и
/// перечисления, и тумблеры, и палитры цветов, и ступени числового ряда, и
/// свести их к одному списку значило бы завести ещё один тип на каждую.
///
/// Не компонент, в отличие от [`SliderBinding`]: наблюдатель клика забирает
/// привязку замыканием, а синхронизации хватает подписи.
pub struct CycleBinding<R: Knobbed> {
    pub cycle: fn(&mut R),
    pub text: fn(&R) -> String,
}

impl<R: Knobbed> Clone for CycleBinding<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Knobbed> Copy for CycleBinding<R> {}

/// Строка-кнопка, привязанная к полю ресурса. Возвращает строку — панель
/// довешивает на неё свои метки (свотч цвета, секция, видимость).
pub fn spawn_cycle_row<R: Knobbed>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    left_px: f32,
    resource: &R,
    binding: CycleBinding<R>,
) -> Entity {
    spawn_value_row(
        commands,
        panel,
        label,
        left_px,
        CycleValueLabel(binding),
        (binding.text)(resource),
        move |_activate: On<Activate>, mut resource: ResMut<R>| (binding.cycle)(&mut resource),
    )
}

/// Подпись строки-кнопки: по ней синхронизация находит текст, который надо
/// перечитать из ресурса.
#[derive(Component)]
pub struct CycleValueLabel<R: Knobbed>(pub CycleBinding<R>);

impl<R: Knobbed> Clone for CycleValueLabel<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Knobbed> Copy for CycleValueLabel<R> {}

fn sync_cycle_values<R: Knobbed>(
    resource: Res<R>,
    mut labels: Query<(&CycleValueLabel<R>, &mut Text)>,
) {
    for (label, mut text) in &mut labels {
        let next = (label.0.text)(&resource);
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Типы, чьи ручки уже зарегистрированы, — чтобы [`AddKnobsExt::add_knobs`]
/// сдержал своё обещание «по разу на ресурс».
///
/// Ресурс правят ручки **разных** панелей (`HumanStyle` — и разброс скоростей
/// в Human, и радиус тела в Navigation; `PolymeshDebug` — радиус агента), и
/// каждая панель обязана регистрировать то, чем пользуется: полагаться на то,
/// что сосед уже позвал, — то самое зеркало списка, от которого уходим. Без
/// этой памяти вторая регистрация просто добавила бы вторую копию систем.
#[derive(Resource, Default)]
struct RegisteredKnobs(std::collections::HashSet<std::any::TypeId>);

/// Регистрация ручек ресурса — по разу на ресурс, независимо от того, сколько
/// у него ручек и по скольким панелям они разложены.
pub trait AddKnobsExt {
    fn add_knobs<R: Knobbed>(&mut self) -> &mut Self;
}

impl AddKnobsExt for App {
    fn add_knobs<R: Knobbed>(&mut self) -> &mut Self {
        if !self
            .world_mut()
            .get_resource_or_insert_with(RegisteredKnobs::default)
            .0
            .insert(std::any::TypeId::of::<R>())
        {
            return self;
        }
        self.add_systems(
            Update,
            (sync_knob_values::<R>, sync_cycle_values::<R>)
                .run_if(resource_exists_and_changed::<R>),
        )
    }
}
