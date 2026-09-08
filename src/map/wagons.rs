//! Вагоны, стоящие на станционных путях.
//!
//! Станционный парк на снимке — это не пустые нитки рельсов, а **составы**:
//! половина площади любого узла занята стоящими вагонами, и без них горловина
//! читается как схема, а не как фотография. Приём тот же, что с машинами
//! ([`super::cars`]), и разница ровно в том, где им стоять.
//!
//! Вагоны ставятся **только на служебные пути** (`service=siding|yard|spur`):
//! на главном ходу состав либо идёт, либо его там нет, а на подъездном он
//! стоит неделями. Это не приближение — это то, что различает станцию и
//! перегон на любом снимке.
//!
//! Ставятся **сцепами**: несколько вагонов подряд без зазора, потом пустой
//! кусок пути. Ровный ряд через равные промежутки выглядел бы как забор.
//!
//! Как и машины, вагоны — **декорация**: ни навмеша, ни симуляции.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RailKind, RailLine};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};
use crate::settings::{WAGON_MAX_ZOOM, Z_WAGON};

/// Габарит четырёхосного вагона, м: полувагон 13.9 × 3.1, высота по борту
/// 3.8 — она и даёт длину тени, тем же котангенсом высоты солнца, что у домов.
const WAGON_LENGTH: f32 = 13.9;
const WAGON_WIDTH: f32 = 3.1;
const WAGON_HEIGHT: f32 = 3.8;
/// Зазор между вагонами в сцепе, м: автосцепка.
const COUPLED_GAP: f32 = 0.9;
/// Сцеп — столько вагонов подряд.
const RAKE_MIN: f32 = 3.0;
const RAKE_MAX: f32 = 16.0;
/// И столько метров пустого пути после него.
const GAP_MIN: f32 = 12.0;
const GAP_MAX: f32 = 90.0;
/// Ближе этого к торцу пути не ставят: там стрелка.
const END_MARGIN: f32 = 12.0;
/// Путь короче этого сцепа не держит.
const TRACK_MIN: f32 = 40.0;

/// Палитра кузовов: полувагон в ржавчине, крытый в сурике, цистерна светлая,
/// хоппер серый. Доли — как на снимке любого узла: половина ряда бурая.
const WAGON_COLORS: [Color; 8] = [
    Color::srgb(0.38, 0.25, 0.20),
    Color::srgb(0.42, 0.28, 0.22),
    Color::srgb(0.33, 0.22, 0.19),
    Color::srgb(0.45, 0.31, 0.25),
    Color::srgb(0.30, 0.33, 0.36),
    Color::srgb(0.55, 0.56, 0.57),
    Color::srgb(0.26, 0.34, 0.31),
    Color::srgb(0.40, 0.41, 0.43),
];

/// Слой вагонов — своя метка, чтобы ступень зума пересобирала только его.
#[derive(Component)]
pub struct WagonLayerTag;

/// Ступени зума: вагон вчетверо длиннее машины и виден дальше, поэтому порог
/// свой, а не общий с [`super::cars`].
pub enum WagonLods {}

impl ZoomLods for WagonLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [WAGON_MAX_ZOOM, f32::INFINITY].into_iter()
    }
}

pub type WagonZoomBucket = ZoomBucket<WagonLods>;

/// Вагон: середина кузова и направление пути под ним.
struct Wagon {
    at: Vec2,
    along: Vec2,
    color: Color,
}

/// ГПСЧ Лемера — тот же, что расставляет машины и кроны.
struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self((seed % 0x7FFF_FFFF).max(1))
    }

    fn next_f32(&mut self) -> f32 {
        self.0 = ((u64::from(self.0) * 48271) % 0x7FFF_FFFF) as u32;
        self.0 as f32 / 2_147_483_647.0
    }

    fn range(&mut self, from: f32, to: f32) -> f32 {
        from + self.next_f32() * (to - from)
    }
}

/// Пересборка слоя: по ступени зума и по смене солнца (у вагона своя тень).
pub fn rebuild_wagons(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<WagonZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<WagonLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if bucket.index > 0 {
        return;
    }
    let wagons = stable_wagons(&map.rails);
    let builder = mesh_wagons(&wagons);
    let count = wagons.len();
    let vertices = builder.vertex_count();
    if builder.is_empty() {
        return;
    }
    // как и у машин: тень полупрозрачна, кузов нет
    let material = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    surface::spawn_layer(
        &mut commands,
        &mut meshes,
        builder,
        Z_WAGON,
        "wagons",
        LayerMaterial::Flat(material),
        WagonLayerTag,
    );
    info!("wagons: {count} standing ({vertices} verts)");
}

/// Составы на всех служебных путях.
fn stable_wagons(rails: &[RailLine]) -> Vec<Wagon> {
    let mut wagons = Vec::new();
    for rail in rails {
        // трамвай и заброшенный путь состава не держат: по первому ходят
        // вагоны другого рода, второй разобран
        if rail.kind != RailKind::Active || !rail.service {
            continue;
        }
        let mut rng = Lcg::new(track_seed(rail));
        stand_along(&mut wagons, rail, &mut rng);
    }
    wagons
}

/// Сцепы вдоль одного пути: сцеп, пустой кусок, сцеп.
fn stand_along(wagons: &mut Vec<Wagon>, rail: &RailLine, rng: &mut Lcg) {
    for pair in rail.points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let length = from.distance(to);
        if length < TRACK_MIN {
            continue;
        }
        let pitch = WAGON_LENGTH + COUPLED_GAP;
        let mut at = END_MARGIN + rng.range(0.0, GAP_MAX);
        while at + WAGON_LENGTH <= length - END_MARGIN {
            let rake = rng.range(RAKE_MIN, RAKE_MAX) as usize;
            for _ in 0..rake {
                if at + WAGON_LENGTH > length - END_MARGIN {
                    break;
                }
                let color = WAGON_COLORS
                    [(rng.next_f32() * WAGON_COLORS.len() as f32) as usize % WAGON_COLORS.len()];
                wagons.push(Wagon {
                    at: from + along * (at + WAGON_LENGTH / 2.0),
                    along,
                    color,
                });
                at += pitch;
            }
            at += rng.range(GAP_MIN, GAP_MAX);
        }
    }
}

/// Все тени, потом все кузова: иначе тень вагона легла бы на соседний.
fn mesh_wagons(wagons: &[Wagon]) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let shadow = SHADOW_COLOR.to_linear();
    let offset = shadow_dir() * (WAGON_HEIGHT * shadow_length_scale());
    for wagon in wagons {
        builder.push_quad(body(wagon, offset), shadow);
    }
    for wagon in wagons {
        builder.push_quad(body(wagon, Vec2::ZERO), wagon.color.to_linear());
    }
    builder
}

fn body(wagon: &Wagon, offset: Vec2) -> [Vec2; 4] {
    let half_length = wagon.along * (WAGON_LENGTH / 2.0);
    let half_width = Vec2::new(-wagon.along.y, wagon.along.x) * (WAGON_WIDTH / 2.0);
    let at = wagon.at + offset;
    [
        at - half_length - half_width,
        at + half_length - half_width,
        at + half_length + half_width,
        at - half_length + half_width,
    ]
}

/// Посев пути — от его первой точки, как у улиц и домов.
fn track_seed(rail: &RailLine) -> u32 {
    let point = rail.points.first().copied().unwrap_or(Vec2::ZERO);
    let x = (point.x * 100.0) as i32 as u32;
    let y = (point.y * 100.0) as i32 as u32;
    let mut hash = x ^ y.rotate_left(16);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(service: bool, length: f32) -> RailLine {
        RailLine {
            points: vec![Vec2::new(100.0, 100.0), Vec2::new(100.0 + length, 100.0)],
            width: 5.0,
            kind: RailKind::Active,
            service,
        }
    }

    /// На служебном пути стоят составы, на главном ходу — нет.
    #[test]
    fn wagons_stand_on_service_track_only() {
        assert!(!stable_wagons(&[track(true, 400.0)]).is_empty());
        assert!(stable_wagons(&[track(false, 400.0)]).is_empty());
    }

    /// Заброшенный путь состава не держит.
    #[test]
    fn a_disused_track_stands_empty() {
        let mut rail = track(true, 400.0);
        rail.kind = RailKind::Disused;
        assert!(stable_wagons(&[rail]).is_empty());
    }

    /// Короткий тупик — тоже: там негде.
    #[test]
    fn a_short_stub_stands_empty() {
        assert!(stable_wagons(&[track(true, 30.0)]).is_empty());
    }

    /// Вагоны идут сцепами: между соседними в сцепе — автосцепка, а не
    /// произвольный зазор.
    #[test]
    fn wagons_come_in_rakes() {
        let wagons = stable_wagons(&[track(true, 600.0)]);
        let mut coupled = 0;
        for pair in wagons.windows(2) {
            let gap = pair[1].at.distance(pair[0].at);
            if (gap - (WAGON_LENGTH + COUPLED_GAP)).abs() < 1e-3 {
                coupled += 1;
            }
        }
        assert!(
            coupled > wagons.len() / 2,
            "{coupled} сцепленных из {}",
            wagons.len()
        );
    }
}
