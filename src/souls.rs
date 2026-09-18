//! Души и призыв: один ресурс, одна трата — душа за сожранного, демон за
//! души. `Souls` — состояние прогона (сброс на `WorldStarted`, в отпечатке);
//! `SummonRequested` — сообщение, которое пишут кнопки HUD, хоткеи `1`/`2` и
//! BRP, а читает `demon::summon` в слоте спавнера. Модель — скилл
//! `city-siege`.

use bevy::ecs::reflect::ReflectMessage;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::demon::DemonKind;
use crate::loading::WorldStarted;
use crate::settings::{SUMMON_COST_BRUTE, SUMMON_COST_GROWTH, SUMMON_COST_IMP};

/// Души: заработано убийствами, потрачено призывом. Инвариант
/// `earned == Telemetry::killed` — обе пишет один обсервер
/// (`demon::behavior::on_demon_caught_human`).
#[derive(Resource, Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct Souls {
    pub earned: u32,
    pub spent: u32,
}

impl Souls {
    pub fn available(&self) -> u32 {
        self.earned.saturating_sub(self.spent)
    }
}

/// Просьба призвать демона. Сообщение, а не событие: пишут его кнопки и
/// хоткеи из `Update`, а читает симуляция на тике, и между ними оно ждёт в
/// буфере. `Default` — ради `brp msg SummonRequested '{"kind":"Brute"}'`.
#[derive(Message, Reflect, Default, Debug, Clone, Copy)]
#[reflect(Message, Default)]
pub struct SummonRequested {
    pub kind: DemonKind,
}

/// Цена призыва: базовая по виду, плюс `SUMMON_COST_GROWTH` за каждого
/// живого демона того же вида — вторая сотня Бесов дороже первой.
pub fn summon_cost(kind: DemonKind, alive_of_kind: usize) -> u32 {
    let base = match kind {
        DemonKind::Imp => SUMMON_COST_IMP,
        DemonKind::Brute => SUMMON_COST_BRUTE,
    };
    (base * (1.0 + SUMMON_COST_GROWTH * alive_of_kind as f32)).ceil() as u32
}

fn on_world_started(_event: On<WorldStarted>, mut souls: ResMut<Souls>) {
    *souls = Souls::default();
}

/// Хоткеи призыва: `1` — Бес, `2` — Громила. Гейт `typing_in_text_input`
/// обязателен: поле seed во вкладке Sim принимает цифры, и без него каждая
/// «1» в seed'е призывала бы Беса.
fn summon_imp(mut requests: MessageWriter<SummonRequested>) {
    requests.write(SummonRequested {
        kind: DemonKind::Imp,
    });
}

fn summon_brute(mut requests: MessageWriter<SummonRequested>) {
    requests.write(SummonRequested {
        kind: DemonKind::Brute,
    });
}

pub struct SoulsPlugin;

impl Plugin for SoulsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Souls>()
            .register_type::<SummonRequested>()
            .init_resource::<Souls>()
            .add_message::<SummonRequested>()
            .add_observer(on_world_started)
            .add_systems(
                Update,
                (
                    summon_imp.run_if(input_just_pressed(KeyCode::Digit1)),
                    summon_brute.run_if(input_just_pressed(KeyCode::Digit2)),
                )
                    .run_if(not(crate::ui::typing_in_text_input)),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_price_grows_with_the_living_of_that_kind() {
        assert_eq!(summon_cost(DemonKind::Imp, 0), SUMMON_COST_IMP as u32);
        assert_eq!(summon_cost(DemonKind::Brute, 0), SUMMON_COST_BRUTE as u32);
        assert!(summon_cost(DemonKind::Imp, 20) > summon_cost(DemonKind::Imp, 0));
        let souls = Souls {
            earned: 10,
            spent: 12,
        };
        assert_eq!(
            souls.available(),
            0,
            "spent past earned never goes negative"
        );
    }
}
