//! Осада на карте — то, что игрок M1 должен видеть, не открывая вкладку Debug:
//!
//! - **территория** — один спрайт на всю карту в шаг растра меток районов:
//!   осквернённые районы фиолетовым, растущие — бледнее по прогрессу, район,
//!   который держит стоящий бастион, — янтарём, район сердца — золотом;
//! - **здоровье бастионов** — полоска над маркером, пока бастион ранен;
//! - **фронт** — кольцо вокруг бастионов, в которые упирается скверна;
//! - **линии осады** — стрелка от Громилы к бастиону, который он ломает.
//!
//! Секция Siege вкладки Sim включает каждое по отдельности; все четыре —
//! чистая косметика в `Update`, симуляция о них не знает. Отладочный слой
//! районов (`ui/debug/overlays.rs`, клавиша T) остаётся: он красит районы
//! разными оттенками, чтобы видеть их границы, а этот слой — ход осады.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::bastion::{Bastion, BastionsStanding, RuinTag};
use crate::combat::{AttackTarget, Health};
use crate::corruption::Corruption;
use crate::demon::{BruteTag, Demon};
use crate::district::{DistrictId, Districts};
use crate::loading::AppState;
use crate::movement::SimPosition;
use crate::prefs::TrackPrefExt;
use crate::settings::{BASTION_MARKER_SIZE, DISTRICT_LABEL_METERS, MAP_SIZE, Z_TERRITORY};
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{UiBuildSet, panel_title};

/// Что из осады рисуется на карте. Всё включено: это слой игры, а не отладки.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "siege")]
pub struct SiegeView {
    pub territory: bool,
    pub health_bars: bool,
    pub front: bool,
    pub siege_lines: bool,
}

impl Default for SiegeView {
    fn default() -> Self {
        Self {
            territory: true,
            health_bars: true,
            front: true,
            siege_lines: true,
        }
    }
}

/// Осквернённый район — тот же фиолетовый, что у вихря портала: скверна
/// приходит оттуда. Непрозрачность держит крыши под слоем различимыми.
const VILE: Srgba = Srgba::rgb(0.40, 0.06, 0.50);
const VILE_ALPHA: f32 = 0.42;
/// Растущий район: от едва заметного до почти осквернённого.
const GROWING_ALPHA_MIN: f32 = 0.08;
const GROWING_ALPHA_SPAN: f32 = 0.26;
/// Район, который держит стоящий бастион: скверна рядом, но не входит.
const HELD: Srgba = Srgba::rgb(0.98, 0.62, 0.12);
const HELD_ALPHA: f32 = 0.20;
/// Район сердца, пока цел, — цвет маркера сердца (`portal.rs`).
const HEART: Srgba = Srgba::rgb(1.0, 0.85, 0.2);
const HEART_ALPHA: f32 = 0.18;
/// Ступеней прогресса, различимых на слое: текстура пересобирается, только
/// когда какой-то район перешёл ступень.
const TERRITORY_SHADES: f32 = 16.0;
/// Не чаще раза в столько секунд реального времени: на 30× районы переходят
/// ступени почти каждый кадр, а глазу хватает четырёх обновлений в секунду.
const TERRITORY_REBUILD_SECS: f32 = 0.25;

/// Полоска здоровья: ширина и толщина в метрах карты, подъём над центром
/// маркера. Шире маркера — чтобы край читался и у целого на треть бастиона.
const BAR_WIDTH: f32 = BASTION_MARKER_SIZE * 1.5;
const BAR_HEIGHT: f32 = 4.0;
const BAR_LIFT: f32 = BASTION_MARKER_SIZE * 0.5 + BAR_HEIGHT;
const BAR_BACK: Color = Color::srgba(0.08, 0.06, 0.10, 0.85);
const BAR_FULL: Srgba = Srgba::rgb(0.35, 0.85, 0.40);
const BAR_EMPTY: Srgba = Srgba::rgb(0.90, 0.20, 0.15);
/// Кольцо фронта вокруг маркера и его цвет — янтарь «держит», как на слое.
const FRONT_RING_RADIUS: f32 = BASTION_MARKER_SIZE * 0.95;
const FRONT_RING: Color = Color::srgb(0.98, 0.62, 0.12);
/// Линия осады: тёмно-малиновая, как кольцо оттенков Громилы.
const SIEGE_LINE: Color = Color::srgb(0.80, 0.18, 0.55);
const SIEGE_ARROW_TIP: f32 = 4.0;

pub struct UiSiegePlugin;

impl Plugin for UiSiegePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SiegeView>()
            .init_resource::<SiegeView>()
            .track_pref::<SiegeView>()
            .add_knobs::<SiegeView>()
            .add_observer(attach_health_bar)
            .add_systems(Startup, build_siege_section.in_set(UiBuildSet::Sections))
            .add_systems(
                Update,
                (
                    sync_territory_layer,
                    sync_health_bars,
                    draw_front.run_if(|view: Res<SiegeView>| view.front),
                    draw_siege_lines.run_if(|view: Res<SiegeView>| view.siege_lines),
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn build_siege_section(mut commands: Commands, panes: Res<SettingsPanes>, view: Res<SiegeView>) {
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Sim),
        SectionSlot::Siege,
        panel_title("Siege"),
        "siege_section",
    );
    let rows: [(&str, CycleBinding<SiegeView>); 4] = [
        (
            "Territory",
            CycleBinding {
                cycle: |view| view.territory = !view.territory,
                text: |view| on_off(view.territory).to_string(),
            },
        ),
        (
            "Health bars",
            CycleBinding {
                cycle: |view| view.health_bars = !view.health_bars,
                text: |view| on_off(view.health_bars).to_string(),
            },
        ),
        (
            "Front",
            CycleBinding {
                cycle: |view| view.front = !view.front,
                text: |view| on_off(view.front).to_string(),
            },
        ),
        (
            "Siege lines",
            CycleBinding {
                cycle: |view| view.siege_lines = !view.siege_lines,
                text: |view| on_off(view.siege_lines).to_string(),
            },
        ),
    ];
    for (label, binding) in rows {
        spawn_cycle_row(&mut commands, panel, label, ROW_LEFT_PX, &*view, binding);
    }
}

// ─── территория ────────────────────────────────────────────────────────────

/// Слой территории: тексель → район считается раз на мир, ключ — под какое
/// состояние осады текстура нарисована.
#[derive(Component)]
struct TerritoryLayer {
    labels: Vec<Option<DistrictId>>,
    key: u64,
}

/// Во что красится район на слое. Чистая функция: порядок проверок и есть
/// правило — осквернён, растёт, держит бастион, сердце, ничего.
fn territory_color(progress: f32, held: bool, heart: bool) -> Option<Srgba> {
    if progress >= 1.0 {
        Some(VILE.with_alpha(VILE_ALPHA))
    } else if progress > 0.0 {
        Some(VILE.with_alpha(GROWING_ALPHA_MIN + GROWING_ALPHA_SPAN * progress))
    } else if held {
        Some(HELD.with_alpha(HELD_ALPHA))
    } else if heart {
        Some(HEART.with_alpha(HEART_ALPHA))
    } else {
        None
    }
}

/// Район «держит бастион»: на фронте и в нём хоть один бастион стоит.
fn is_held(
    corruption: &Corruption,
    districts: &Districts,
    standing: &BastionsStanding,
    district: DistrictId,
) -> bool {
    standing
        .0
        .get(district as usize)
        .is_some_and(|&count| count > 0)
        && corruption.on_front(districts, district)
}

/// Ключ состояния слоя, FNV-1a: ступень прогресса и «держит» по каждому району.
fn territory_key(
    corruption: &Corruption,
    districts: &Districts,
    standing: &BastionsStanding,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for id in 0..districts.len() {
        let progress = corruption.progress.get(id).copied().unwrap_or(0.0);
        let shade = (progress.clamp(0.0, 1.0) * TERRITORY_SHADES) as u64;
        let held = u64::from(is_held(corruption, districts, standing, id as DistrictId));
        hash ^= shade << 1 | held;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[allow(clippy::too_many_arguments)]
fn sync_territory_layer(
    mut commands: Commands,
    view: Res<SiegeView>,
    districts: Res<Districts>,
    corruption: Res<Corruption>,
    standing: Res<BastionsStanding>,
    time: Res<Time<Real>>,
    mut last_rebuild: Local<f32>,
    mut images: ResMut<Assets<Image>>,
    mut layers: Query<(Entity, &mut TerritoryLayer, &mut Sprite)>,
) {
    if !view.territory || districts.is_empty() {
        for (entity, _, sprite) in &layers {
            images.remove(&sprite.image);
            commands.entity(entity).despawn();
        }
        return;
    }
    let size = (MAP_SIZE / DISTRICT_LABEL_METERS).ceil().as_uvec2();

    // новый мир — новые районы: старый слой и его разметка уходят целиком
    if districts.is_changed() {
        for (entity, _, sprite) in &layers {
            images.remove(&sprite.image);
            commands.entity(entity).despawn();
        }
    }
    let key = territory_key(&corruption, &districts, &standing);
    let now = time.elapsed_secs();

    let Ok((_, mut layer, mut sprite)) = layers.single_mut() else {
        if districts.is_changed() || layers.is_empty() {
            let labels = texel_districts(&districts, size);
            let image = territory_image(&labels, size, &districts, &corruption, &standing);
            commands.spawn((
                TerritoryLayer { labels, key },
                Sprite {
                    image: images.add(image),
                    custom_size: Some(MAP_SIZE),
                    ..default()
                },
                Transform::from_translation((MAP_SIZE / 2.0).extend(Z_TERRITORY)),
                DespawnOnExit(AppState::Playing),
                Name::new("siege_territory"),
            ));
            *last_rebuild = now;
        }
        return;
    };
    if districts.is_changed() || layer.key == key || now - *last_rebuild < TERRITORY_REBUILD_SECS {
        return;
    }
    let image = territory_image(&layer.labels, size, &districts, &corruption, &standing);
    images.remove(&sprite.image);
    sprite.image = images.add(image);
    layer.key = key;
    *last_rebuild = now;
}

/// Район под каждым текселем, строка 0 — верх спрайта (максимальный мировой y).
fn texel_districts(districts: &Districts, size: UVec2) -> Vec<Option<DistrictId>> {
    let mut labels = Vec::with_capacity((size.x * size.y) as usize);
    for row in 0..size.y {
        for column in 0..size.x {
            let position = Vec2::new(column as f32 + 0.5, (size.y - 1 - row) as f32 + 0.5)
                * DISTRICT_LABEL_METERS;
            labels.push(districts.district_at(position));
        }
    }
    labels
}

fn territory_image(
    labels: &[Option<DistrictId>],
    size: UVec2,
    districts: &Districts,
    corruption: &Corruption,
    standing: &BastionsStanding,
) -> Image {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0) as u8;
    let heart = districts
        .districts
        .iter()
        .position(|district| district.dist_to_heart == Some(0));
    // цвет — раз на район, а не на тексель
    let colors: Vec<[u8; 4]> = (0..districts.len())
        .map(|id| {
            let progress = corruption.progress.get(id).copied().unwrap_or(0.0);
            let held = is_held(corruption, districts, standing, id as DistrictId);
            match territory_color(progress, held, heart == Some(id)) {
                Some(color) => [
                    byte(color.red),
                    byte(color.green),
                    byte(color.blue),
                    byte(color.alpha),
                ],
                // прозрачный тексель несёт цвет скверны, а не чёрный: линейный
                // сэмплер смешивает соседей, и чёрный дал бы грязную кайму
                None => [byte(VILE.red), byte(VILE.green), byte(VILE.blue), 0],
            }
        })
        .collect();
    let empty = [byte(VILE.red), byte(VILE.green), byte(VILE.blue), 0];
    let mut data = Vec::with_capacity(labels.len() * 4);
    for label in labels {
        let texel = label.map_or(empty, |id| colors[id as usize]);
        data.extend_from_slice(&texel);
    }
    let mut image = Image::new(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // линейный: край района в 8 м — не пиксель-арт, пятно должно быть мягким
    image.sampler = ImageSampler::linear();
    image
}

// ─── здоровье бастионов ────────────────────────────────────────────────────

/// Подложка полоски — дочерняя сущность бастиона; уходит вместе с ним.
#[derive(Component)]
struct HealthBar;

/// Заполнение полоски — дочь подложки.
#[derive(Component)]
struct HealthBarFill;

/// Каждому бастиону — полоска, скрытая, пока он цел.
fn attach_health_bar(event: On<Add, Bastion>, mut commands: Commands) {
    commands.entity(event.entity).with_child((
        HealthBar,
        Sprite {
            color: BAR_BACK,
            custom_size: Some(Vec2::new(BAR_WIDTH, BAR_HEIGHT)),
            ..default()
        },
        Transform::from_xyz(0.0, BAR_LIFT, 0.02),
        Visibility::Hidden,
        Name::new("bastion_health_bar"),
        children![(
            HealthBarFill,
            Sprite {
                color: BAR_FULL.into(),
                custom_size: Some(Vec2::new(BAR_WIDTH, BAR_HEIGHT * 0.6)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.01),
            Name::new("bastion_health_fill"),
        )],
    ));
}

/// Доля здоровья → ширина и цвет заполнения; полоска видна, только пока
/// бастион ранен и не руина.
fn sync_health_bars(
    view: Res<SiegeView>,
    bastions: Query<(Ref<Health>, &Children, Has<RuinTag>), With<Bastion>>,
    mut bars: Query<(&mut Visibility, &Children), With<HealthBar>>,
    mut fills: Query<(&mut Sprite, &mut Transform), With<HealthBarFill>>,
) {
    let view_changed = view.is_changed();
    for (health, children, ruined) in &bastions {
        if !view_changed && !health.is_changed() {
            continue;
        }
        let share = if health.max > 0.0 {
            (health.hp / health.max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let shown = view.health_bars && !ruined && share < 1.0;
        for &child in children {
            let Ok((mut visibility, bar_children)) = bars.get_mut(child) else {
                continue;
            };
            visibility.set_if_neq(if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            for &fill in bar_children {
                if let Ok((mut sprite, mut transform)) = fills.get_mut(fill) {
                    let width = BAR_WIDTH * share;
                    sprite.custom_size = Some(Vec2::new(width, BAR_HEIGHT * 0.6));
                    sprite.color = BAR_EMPTY.mix(&BAR_FULL, share).into();
                    // якорь спрайта — центр: сдвигаем, чтобы полоса убывала справа
                    transform.translation.x = (width - BAR_WIDTH) / 2.0;
                }
            }
        }
    }
}

// ─── фронт и осада ─────────────────────────────────────────────────────────

/// Кольцо вокруг бастиона на фронте: сюда упирается скверна и сюда пойдут
/// Громилы. Бастионов десятки — гизмо за кадр ничего не стоят.
fn draw_front(
    mut gizmos: Gizmos,
    districts: Res<Districts>,
    corruption: Res<Corruption>,
    bastions: Query<(&Bastion, &Transform), Without<RuinTag>>,
) {
    for (bastion, transform) in &bastions {
        if corruption.on_front(&districts, bastion.district) {
            gizmos.circle_2d(
                transform.translation.truncate(),
                FRONT_RING_RADIUS,
                FRONT_RING,
            );
        }
    }
}

/// Стрелка от Громилы к бастиону, который он осаждает.
fn draw_siege_lines(
    mut gizmos: Gizmos,
    brutes: Query<(&SimPosition, &AttackTarget), (With<Demon>, With<BruteTag>)>,
    targets: Query<&Transform, With<Bastion>>,
) {
    for (position, target) in &brutes {
        let Ok(transform) = targets.get(target.0) else {
            continue;
        };
        let to = transform.translation.truncate();
        let distance = position.0.distance(to);
        gizmos
            .arrow_2d(position.0, to, SIEGE_LINE)
            .with_tip_length(SIEGE_ARROW_TIP.min(distance * 0.5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corruption_outranks_the_bastion_and_the_heart() {
        let corrupted = territory_color(1.0, true, true).unwrap();
        assert_eq!(corrupted.alpha, VILE_ALPHA);
        let growing = territory_color(0.5, true, true).unwrap();
        assert!(growing.alpha > GROWING_ALPHA_MIN && growing.alpha < VILE_ALPHA);
        assert_eq!(territory_color(0.0, true, true).unwrap().red, HELD.red);
        assert_eq!(
            territory_color(0.0, false, true).unwrap().green,
            HEART.green
        );
        assert!(territory_color(0.0, false, false).is_none());
    }

    #[test]
    fn growth_brightens_monotonically() {
        let alphas: Vec<f32> = [0.1, 0.4, 0.7, 0.99]
            .iter()
            .map(|&progress| territory_color(progress, false, false).unwrap().alpha)
            .collect();
        assert!(
            alphas.windows(2).all(|pair| pair[0] < pair[1]),
            "{alphas:?}"
        );
    }
}
