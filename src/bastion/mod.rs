//! Бастионы: опорные пункты людей. Пока стоит — скверна в район не идёт;
//! сломанный остаётся руиной. Откуда берутся точки — `plan_sites` в потоке
//! загрузки (теги OSM плюс добор до квоты района, `fill.rs`); сущности
//! спавнятся по ним на входе в мир, здоровье носят из `combat.rs`, руина —
//! тот же entity с `RuinTag`, рестарт лечит на месте. Модель и её замеры —
//! скилл `city-siege`.

pub mod fill;

use bevy::prelude::*;

use self::fill::{Candidate, closeness, fill_quota};
use crate::combat::{Destroyed, Health};
use crate::district::{DistrictId, Districts};
use crate::loading::{AppState, WorldInitSet, WorldStarted};
use crate::map::osm::model::{BastionKind, MapData, ring_area, ring_centroid};
use crate::navigation::{Navmesh, nearest_tile_where};
use crate::settings::{
    BASTION_DISTRICT_REACH, BASTION_HEART_GAIN, BASTION_HP, BASTION_MARKER_SIZE,
    BASTION_SNAP_METERS, STRONGHOLD_MIN_AREA, Z_BASTION,
};

/// Бастион на карте: вид и район. Руина — тот же entity с [`RuinTag`], сам
/// `Bastion` остаётся: перепись «сколько стоит» ведёт [`BastionsStanding`], а
/// запросы, которым нужны только целые, фильтруют `Without<RuinTag>`.
#[derive(Component, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Bastion {
    pub kind: BastionKind,
    pub district: DistrictId,
}

/// Сломанный бастион: спрайт погашен, скверну не держит, `DespawnOnExit`
/// остаётся — как труп у человека (`human::to_corpse`).
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct RuinTag;

/// Сколько бастионов стоит в каждом районе — состояние прогона, не поле
/// `Districts`: заполняется на `WorldStarted` из [`BastionSites`] (все целы),
/// убывает по [`BastionDestroyed`], входит в отпечаток прогона
/// (`determinism::fingerprint`). Читает её скверна.
#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct BastionsStanding(pub Vec<u16>);

/// Бастион стал руиной.
#[derive(Event, Debug, Clone, Copy)]
pub struct BastionDestroyed {
    pub entity: Entity,
    pub district: DistrictId,
}

/// Запас прочности по близости к сердцу: [`BASTION_HP`] на окраине, до
/// `× (1 + BASTION_HEART_GAIN)` у сердца. Множители по виду — M2.
pub fn bastion_hp(closeness: f32) -> f32 {
    BASTION_HP * (1.0 + BASTION_HEART_GAIN * closeness)
}

/// Цвет маркера по виду: подпись бастиона в M1 — только цвет.
fn kind_color(kind: BastionKind) -> Color {
    match kind {
        BastionKind::Police => Color::srgb(0.20, 0.45, 0.95),
        BastionKind::FireStation => Color::srgb(0.95, 0.35, 0.15),
        BastionKind::Church => Color::srgb(0.95, 0.80, 0.25),
        BastionKind::Military => Color::srgb(0.45, 0.60, 0.25),
        BastionKind::Stronghold => Color::srgb(0.60, 0.60, 0.65),
    }
}

/// Руина — погашенный маркер: тёмный, полупрозрачный, но на месте.
const RUIN_COLOR: Color = Color::srgba(0.25, 0.22, 0.22, 0.6);

/// Спавн бастионов по местам из [`BastionSites`]: маркер-спрайт над крышами,
/// здоровье по близости к сердцу.
fn spawn_bastions(mut commands: Commands, sites: Res<BastionSites>) {
    for site in &sites.sites {
        commands.spawn((
            Sprite {
                color: kind_color(site.kind),
                custom_size: Some(Vec2::splat(BASTION_MARKER_SIZE)),
                ..default()
            },
            Transform::from_translation(site.pos.extend(Z_BASTION)),
            Bastion {
                kind: site.kind,
                district: site.district,
            },
            Health::full(bastion_hp(site.closeness)),
            DespawnOnExit(AppState::Playing),
            Name::new("bastion"),
        ));
    }
}

/// Новый прогон — все бастионы целы, на месте: здоровье в максимум,
/// `RuinTag` снят, маркер зажжён (решение 11 `ROADMAP.md`: рестарт не
/// пересоздаёт их, список despawn'а в `restart.rs` не растёт). Заодно
/// [`BastionsStanding`] — по местам, все стоят.
fn on_world_started(
    _event: On<WorldStarted>,
    mut commands: Commands,
    sites: Res<BastionSites>,
    districts: Res<Districts>,
    mut standing: ResMut<BastionsStanding>,
    mut bastions: Query<(Entity, &Bastion, &mut Health, &mut Sprite, Has<RuinTag>)>,
) {
    standing.0 = vec![0; districts.len()];
    for site in &sites.sites {
        standing.0[site.district as usize] += 1;
    }
    for (entity, bastion, mut health, mut sprite, ruined) in &mut bastions {
        health.heal();
        if ruined {
            commands.entity(entity).remove::<RuinTag>();
            sprite.color = kind_color(bastion.kind);
        }
    }
}

/// Бастион добит: руина на месте, район теряет одного стоящего.
fn on_destroyed(
    event: On<Destroyed>,
    mut commands: Commands,
    mut standing: ResMut<BastionsStanding>,
    mut bastions: Query<(&Bastion, &mut Sprite), Without<RuinTag>>,
) {
    let Ok((bastion, mut sprite)) = bastions.get_mut(event.entity) else {
        return;
    };
    if let Some(count) = standing.0.get_mut(bastion.district as usize) {
        *count = count.saturating_sub(1);
    }
    sprite.color = RUIN_COLOR;
    commands.entity(event.entity).insert(RuinTag);
    commands.trigger(BastionDestroyed {
        entity: event.entity,
        district: bastion.district,
    });
}

/// Место бастиона: точка на проходимом тайле у стены, район, близость к
/// сердцу (от неё — квота района и запас прочности).
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct BastionSite {
    pub pos: Vec2,
    pub kind: BastionKind,
    pub district: DistrictId,
    /// 1 у сердца, 0 у самого дальнего района.
    pub closeness: f32,
}

/// Все места бастионов карты — производное от карты, как районы: считается в
/// потоке загрузки, рестарт не трогает. Теги впереди, добор за ними.
#[derive(Resource, Debug, Default, Reflect)]
#[reflect(Resource)]
pub struct BastionSites {
    pub sites: Vec<BastionSite>,
    /// Сколько из `sites` пришло из тегов OSM; остальные — `Stronghold`.
    pub tagged: usize,
    /// Бастионов из тегов, у которых рядом нет проходимого тайла в районе.
    pub dropped: usize,
}

impl BastionSites {
    pub fn strongholds(&self) -> usize {
        self.sites.len() - self.tagged
    }
}

/// Раскладка бастионов по пропрунённому navmesh и районам. Точка бастиона
/// — не центроид здания, а ближайший к нему проходимый тайл: центроид лежит
/// внутри здания, куда ни пешке не дойти, ни району не дотянуться. Район —
/// по снапнутой точке; растр меток грубее навтайла, так что ищется ближайшая
/// размеченная ячейка ([`Districts::district_near`]).
pub fn plan_sites(map: &MapData, districts: &Districts, navmesh: &Navmesh) -> BastionSites {
    let max_dist = districts
        .districts
        .iter()
        .filter_map(|district| district.dist_to_heart)
        .max()
        .unwrap_or(0);
    let closeness: Vec<f32> = districts
        .districts
        .iter()
        .map(|district| closeness(district.dist_to_heart, max_dist))
        .collect();

    let snap_tiles = (BASTION_SNAP_METERS / navmesh.tile_size) as i32;
    let place = |position: Vec2| -> Option<(Vec2, DistrictId)> {
        let tile = nearest_tile_where(navmesh.to_tile(position), snap_tiles, |tile| {
            navmesh.is_passable(tile.x, tile.y)
        })?;
        let pos = navmesh.tile_center(tile);
        let district = districts.district_near(pos, BASTION_DISTRICT_REACH)?;
        Some((pos, district))
    };

    let mut dropped = 0;
    let mut sites: Vec<BastionSite> = map
        .bastions
        .iter()
        .filter_map(|bastion| {
            let Some((pos, district)) = place(bastion.pos) else {
                dropped += 1;
                return None;
            };
            Some(BastionSite {
                pos,
                kind: bastion.kind,
                district,
                closeness: closeness[district as usize],
            })
        })
        .collect();
    let tagged = sites.len();

    let candidates: Vec<Candidate> = map
        .buildings
        .iter()
        .filter(|building| ring_area(&building.outer) >= STRONGHOLD_MIN_AREA)
        .filter_map(|building| {
            let (pos, district) = place(ring_centroid(&building.outer))?;
            Some(Candidate {
                pos,
                seed: building.outer[0],
                district,
            })
        })
        .collect();
    sites.extend(fill_quota(&sites, &candidates, &closeness));

    BastionSites {
        sites,
        tagged,
        dropped,
    }
}

pub struct BastionPlugin;

impl Plugin for BastionPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<BastionSites>()
            .register_type::<Bastion>()
            .register_type::<RuinTag>()
            .register_type::<BastionsStanding>()
            .init_resource::<BastionSites>()
            .init_resource::<BastionsStanding>()
            .add_observer(on_world_started)
            .add_observer(on_destroyed)
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_bastions.in_set(WorldInitSet::Spawn),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_grows_fourfold_toward_the_heart() {
        assert_eq!(bastion_hp(0.0), BASTION_HP);
        assert_eq!(bastion_hp(1.0), BASTION_HP * 4.0);
        assert!(bastion_hp(0.5) > bastion_hp(0.0) && bastion_hp(0.5) < bastion_hp(1.0));
    }

    fn app_with_two_bastions() -> App {
        let mut app = App::new();
        app.add_plugins(BastionPlugin);
        let mut districts = Districts::default();
        districts.districts = vec![
            crate::district::District {
                cell: IVec2::ZERO,
                tiles: 1,
                centroid: Vec2::ZERO,
                neighbours: vec![],
                dist_to_heart: Some(0),
            },
            crate::district::District {
                cell: IVec2::X,
                tiles: 1,
                centroid: Vec2::X,
                neighbours: vec![],
                dist_to_heart: Some(1),
            },
        ];
        app.insert_resource(districts);
        let sites = vec![
            BastionSite {
                pos: Vec2::ZERO,
                kind: BastionKind::Police,
                district: 0,
                closeness: 1.0,
            },
            BastionSite {
                pos: Vec2::X,
                kind: BastionKind::Stronghold,
                district: 1,
                closeness: 0.5,
            },
        ];
        app.insert_resource(BastionSites {
            sites,
            tagged: 1,
            dropped: 0,
        });
        app.world_mut().run_system_once(spawn_bastions).unwrap();
        app.world_mut().trigger(WorldStarted);
        app
    }

    use bevy::ecs::system::RunSystemOnce;

    /// Добитый бастион — руина на месте, район теряет стоящего; новый прогон
    /// лечит его там же, без пересоздания.
    #[test]
    fn a_destroyed_bastion_becomes_a_ruin_and_a_new_run_heals_it_in_place() {
        let mut app = app_with_two_bastions();
        assert_eq!(app.world().resource::<BastionsStanding>().0, vec![1, 1]);

        let police = app
            .world_mut()
            .query_filtered::<(Entity, &Bastion), With<Bastion>>()
            .iter(app.world())
            .find(|(_, bastion)| bastion.kind == BastionKind::Police)
            .map(|(entity, _)| entity)
            .expect("police bastion spawned");
        app.world_mut()
            .entity_mut(police)
            .get_mut::<Health>()
            .unwrap()
            .damage(10_000.0);
        // команды обсервера (тег руины) ложатся на ближайшем flush
        app.world_mut().trigger(Destroyed { entity: police });
        app.world_mut().flush();

        assert!(app.world().entity(police).contains::<RuinTag>());
        assert!(app.world().entity(police).contains::<Bastion>());
        assert_eq!(app.world().resource::<BastionsStanding>().0, vec![0, 1]);
        // второй удар по руине ничего не меняет
        app.world_mut().trigger(Destroyed { entity: police });
        app.world_mut().flush();
        assert_eq!(app.world().resource::<BastionsStanding>().0, vec![0, 1]);

        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();
        let entity = app.world().entity(police);
        assert!(!entity.contains::<RuinTag>());
        let health = entity.get::<Health>().unwrap();
        assert_eq!(health.hp, health.max);
        assert_eq!(app.world().resource::<BastionsStanding>().0, vec![1, 1]);
        assert_eq!(
            app.world_mut()
                .query::<&Bastion>()
                .iter(app.world())
                .count(),
            2,
            "a new run heals in place, it does not respawn"
        );
    }
}
