//! Добор бастионов до квоты района: чистая арифметика над уже снапнутыми
//! точками, без navmesh и без ECS — обход карты остался в `plan_sites`.

use bevy::prelude::*;

use super::BastionSite;
use crate::district::DistrictId;
use crate::map::osm::model::BastionKind;
use crate::map::osm::parse::BASTION_DEDUP_METERS;
use crate::rng::lcg_seeded_by;
use crate::settings::BASTION_QUOTA_STEPS;

/// Здание-кандидат в опорные пункты: уже отфильтровано по площади, точка уже
/// снапнута на проходимый тайл у стены, район — по ней.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub pos: Vec2,
    /// Первая вершина контура — семя жребия ([`lcg_seeded_by`]): одна карта
    /// всегда даёт одни и те же опорные пункты, seed мира их не трогает.
    pub seed: Vec2,
    pub district: DistrictId,
}

/// Близость района к сердцу: 1 у сердца, 0 у самого дальнего (или без пути).
pub fn closeness(dist_to_heart: Option<u16>, max_dist: u16) -> f32 {
    match dist_to_heart {
        Some(dist) => 1.0 - dist as f32 / max_dist.max(1) as f32,
        None => 0.0,
    }
}

/// Сколько бастионов положено району по близости — первая ступень
/// [`BASTION_QUOTA_STEPS`], чья верхняя граница не ниже `closeness`.
pub fn quota(closeness: f32) -> u8 {
    BASTION_QUOTA_STEPS
        .iter()
        .find(|(bound, _)| closeness <= *bound)
        .map_or(
            BASTION_QUOTA_STEPS[BASTION_QUOTA_STEPS.len() - 1].1,
            |(_, count)| *count,
        )
}

/// Недостающие до квоты бастионы — `Stronghold` из зданий района: жребий по
/// геометрии, самые «ранние» по нему здания первыми. Здание, в котором уже
/// стоит бастион из тега (точка ближе [`BASTION_DEDUP_METERS`]), не кандидат.
/// Район без подходящих зданий остаётся с тем, что есть. Возвращает только
/// добор; `closeness` — по номеру района.
pub fn fill_quota(
    tagged: &[BastionSite],
    candidates: &[Candidate],
    closeness: &[f32],
) -> Vec<BastionSite> {
    let mut have = vec![0u8; closeness.len()];
    for site in tagged {
        have[site.district as usize] = have[site.district as usize].saturating_add(1);
    }

    let mut by_district: Vec<Vec<(u32, usize)>> = vec![Vec::new(); closeness.len()];
    for (index, candidate) in candidates.iter().enumerate() {
        let taken = tagged.iter().any(|site| {
            site.district == candidate.district
                && site.pos.distance(candidate.pos) < BASTION_DEDUP_METERS
        });
        if taken {
            continue;
        }
        // доля в [0, 1): её биты сравниваются как сама доля
        let key = lcg_seeded_by(candidate.seed)().to_bits();
        by_district[candidate.district as usize].push((key, index));
    }

    let mut strongholds = Vec::new();
    for (district, list) in by_district.iter_mut().enumerate() {
        let need = quota(closeness[district]).saturating_sub(have[district]);
        if need == 0 || list.is_empty() {
            continue;
        }
        list.sort_unstable();
        strongholds.extend(
            list.iter()
                .take(need as usize)
                .map(|&(_, index)| BastionSite {
                    pos: candidates[index].pos,
                    kind: BastionKind::Stronghold,
                    district: district as DistrictId,
                    closeness: closeness[district],
                }),
        );
    }
    strongholds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(pos: Vec2, kind: BastionKind, district: DistrictId) -> BastionSite {
        BastionSite {
            pos,
            kind,
            district,
            closeness: 1.0,
        }
    }

    fn candidate(x: f32, district: DistrictId) -> Candidate {
        Candidate {
            pos: Vec2::new(x, 0.0),
            seed: Vec2::new(x, 0.0),
            district,
        }
    }

    #[test]
    fn quota_steps_by_closeness() {
        assert_eq!(quota(0.0), 0);
        assert_eq!(quota(0.3), 0);
        assert_eq!(quota(0.31), 1);
        assert_eq!(quota(0.7), 1);
        assert_eq!(quota(0.9), 2);
        assert_eq!(quota(1.0), 3);
        assert_eq!(closeness(Some(0), 10), 1.0);
        assert_eq!(closeness(Some(10), 10), 0.0);
        assert_eq!(closeness(None, 10), 0.0);
    }

    /// Район у сердца с двумя тегами и квотой 3 добирает один Stronghold — из
    /// самого «раннего» по жребию здания, и всегда одного и того же.
    #[test]
    fn a_district_short_of_its_quota_takes_the_earliest_building_by_lot() {
        let tagged = [
            site(Vec2::new(10.0, 0.0), BastionKind::Police, 0),
            site(Vec2::new(50.0, 0.0), BastionKind::Church, 0),
        ];
        let candidates = [
            candidate(100.0, 0),
            candidate(200.0, 0),
            candidate(300.0, 0),
        ];
        let earliest = candidates
            .iter()
            .min_by(|a, b| lcg_seeded_by(a.seed)().total_cmp(&lcg_seeded_by(b.seed)()))
            .unwrap();

        let strongholds = fill_quota(&tagged, &candidates, &[1.0]);
        assert_eq!(strongholds.len(), 1);
        assert_eq!(strongholds[0].kind, BastionKind::Stronghold);
        assert_eq!(strongholds[0].pos, earliest.pos);
        assert_eq!(strongholds, fill_quota(&tagged, &candidates, &[1.0]));
    }

    /// Квота 0 — ничего; здание с тегом — не кандидат; зданий меньше, чем
    /// не хватает, — сколько есть.
    #[test]
    fn quota_zero_tagged_buildings_and_scarcity() {
        let far = fill_quota(&[], &[candidate(100.0, 0)], &[0.2]);
        assert!(far.is_empty());

        let tagged = [site(Vec2::new(100.0, 0.0), BastionKind::Police, 0)];
        let only_tagged = fill_quota(&tagged, &[candidate(100.0, 0)], &[1.0]);
        assert!(only_tagged.is_empty(), "{only_tagged:?}");

        let scarce = fill_quota(&[], &[candidate(100.0, 0)], &[1.0]);
        assert_eq!(scarce.len(), 1);
    }
}
