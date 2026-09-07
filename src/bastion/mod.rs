//! Бастионы: опорные пункты людей. Пока стоит — скверна в район не идёт;
//! сломанный остаётся руиной. Откуда берутся точки — здесь (`plan_sites` в
//! потоке загрузки: теги OSM плюс добор до квоты района, `fill.rs`); сами
//! сущности, здоровье и руины — следующие шаги `ROADMAP.md`. Модель и её
//! замеры — скилл `city-siege`.

pub mod fill;

use bevy::prelude::*;

use self::fill::{Candidate, closeness, fill_quota};
use crate::district::{DistrictId, Districts};
use crate::map::osm::model::{BastionKind, MapData, ring_area, ring_centroid};
use crate::navigation::{Navmesh, nearest_tile_where};
use crate::settings::{BASTION_DISTRICT_REACH, BASTION_SNAP_METERS, STRONGHOLD_MIN_AREA};

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
            .init_resource::<BastionSites>();
    }
}
