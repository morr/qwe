//! Пост-обработка кадра: bloom на портале и виньетка по краям экрана.
//!
//! **Bloom.** Камера рендерит в HDR-текстуру ([`bevy::camera::Hdr`]), и всё,
//! что ярче 1.0, расплывается свечением. Ярче единицы на карте только портал:
//! его спрайт умножен на [`PORTAL_GLOW`], а порог префильтра стоит выше
//! 1.0, так что белые дороги и светлые крыши свечения не получают.
//! Тонмаппинг выключен намеренно: любой из готовых кривых перекрасил бы всю
//! палитру карты, а нужен только ореол.
//!
//! **Виньетка.** Полноэкранный UI-узел с радиальным градиентом от
//! прозрачного центра к тёмным углам. Он под панелями (`GlobalZIndex` ниже
//! нуля) и `Pickable::IGNORE`: иначе он был бы «узлом UI под курсором» для
//! `camera::pointer_over_ui`, и карта перестала бы ловить мышь целиком.

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::picking::Pickable;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter};
use bevy::prelude::*;
use bevy::ui::{ColorStop, GlobalZIndex};

/// Множитель цвета спрайта портала: в HDR-кадре он уходит за 1.0 и цветёт.
/// Синий сильнее остальных — ореол того же цвета, что воронка.
pub const PORTAL_GLOW: Color = Color::linear_rgb(2.0, 2.2, 3.6);

/// Сила bloom — доля свечения в кадре; `OLD_SCHOOL` берёт 0.05, здесь
/// ощутимо больше, потому что источник один и маленький, а ореол должен
/// быть виден с общего плана.
const BLOOM_INTENSITY: f32 = 0.4;
/// Затемнение углов виньетки; в центре — ноль.
const VIGNETTE_ALPHA: f32 = 0.22;
/// Где начинается затемнение, % радиуса до дальнего угла.
const VIGNETTE_START_PERCENT: f32 = 55.0;

/// Компоненты камеры, включающие bloom. Спавн камеры — в `camera.rs`;
/// здесь только то, что относится к пост-обработке.
pub fn camera_post_process() -> impl Bundle {
    (
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Bloom {
            intensity: BLOOM_INTENSITY,
            // колено мягкого порога — `threshold × softness` в обе стороны:
            // при 1.0 / 0.4 светилось всё ярче 0.6, то есть белые дороги и
            // светлые крыши, и карта подёргивалась дымкой. 1.1 / 0.1 начинает
            // с 0.99: чисто белая дорога (1.0) вклада не даёт, портал — даёт
            prefilter: BloomPrefilter {
                threshold: 1.1,
                threshold_softness: 0.1,
            },
            composite_mode: BloomCompositeMode::Additive,
            ..Bloom::NATURAL
        },
    )
}

pub struct PostProcessPlugin;

impl Plugin for PostProcessPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_vignette);
    }
}

fn spawn_vignette(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundGradient::from(RadialGradient {
            shape: RadialGradientShape::FarthestCorner,
            position: UiPosition::CENTER,
            stops: vec![
                ColorStop::new(Color::NONE, percent(VIGNETTE_START_PERCENT)),
                ColorStop::new(Color::BLACK.with_alpha(VIGNETTE_ALPHA), percent(100)),
            ],
            ..default()
        }),
        Pickable::IGNORE,
        GlobalZIndex(-1),
        Name::new("vignette"),
    ));
}
