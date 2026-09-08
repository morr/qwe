//! Скверна: поле по районам, растущее от портала цепью соседей, замедляемое
//! людьми и останавливаемое стоящим бастионом. Состояние прогона —
//! сбрасывается на `WorldStarted`, входит в отпечаток прогона. Модель и её
//! замеры — скилл `city-siege`.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::bastion::BastionsStanding;
use crate::determinism::SimPipeline;
use crate::district::{District, DistrictCensus, DistrictId, Districts};
use crate::loading::{PlayPhase, WorldStarted};
use crate::settings::{CORRUPTION_CROWD_HALF, CORRUPTION_RATE};
use crate::spatial::SimSet;

/// Скверна по районам: `progress` 0…1 на район, район **осквернён** при
/// `progress ≥ 1`; `to_heart` — переходов от осквернённого множества до
/// района сердца по графу соседства (бастионы проходимы — их можно сломать),
/// `None` — сердца нет или до него не дойти.
#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct Corruption {
    pub progress: Vec<f32>,
    pub to_heart: Option<u16>,
}

impl Corruption {
    pub fn is_corrupted(&self, district: DistrictId) -> bool {
        self.progress
            .get(district as usize)
            .is_some_and(|&progress| progress >= 1.0)
    }

    pub fn corrupted(&self) -> usize {
        self.progress
            .iter()
            .filter(|&&progress| progress >= 1.0)
            .count()
    }
}

/// Район только что осквернён.
#[derive(Event, Debug, Clone, Copy)]
pub struct DistrictCorrupted {
    pub district: DistrictId,
}

/// Один шаг скверны, чистая функция: для каждого не осквернённого района, у
/// которого хотя бы один сосед осквернён **и** в котором не стоит ни одного
/// бастиона, `progress += dt · CORRUPTION_RATE / (1 + humans /
/// CORRUPTION_CROWD_HALF)`. Соседство читается по снимку начала шага, так что
/// район, осквернённый на этом шаге, заражает дальше со следующего. Обход по
/// номеру района, ГПСЧ нет, районы независимы — детерминизм даром.
/// `humans` и `standing` могут быть короче (перепись ещё не прошла) — тогда
/// ноль. Возвращает районы, осквернённые этим шагом.
pub fn step(
    progress: &mut [f32],
    districts: &[District],
    humans: &[u32],
    standing: &[u16],
    dt: f32,
) -> Vec<DistrictId> {
    let was_corrupted: Vec<bool> = progress.iter().map(|&p| p >= 1.0).collect();
    let mut newly = Vec::new();
    for (index, district) in districts.iter().enumerate() {
        if was_corrupted[index] || standing.get(index).is_some_and(|&count| count > 0) {
            continue;
        }
        let exposed = district
            .neighbours
            .iter()
            .any(|&neighbour| was_corrupted[neighbour as usize]);
        if !exposed {
            continue;
        }
        let crowd = humans.get(index).copied().unwrap_or(0) as f32;
        progress[index] += dt * CORRUPTION_RATE / (1.0 + crowd / CORRUPTION_CROWD_HALF);
        if progress[index] >= 1.0 {
            progress[index] = 1.0;
            newly.push(index as DistrictId);
        }
    }
    newly
}

/// Переходов от осквернённого множества до сердца: BFS по графу соседства от
/// всех осквернённых районов разом. `None` — сердца нет или оно отрезано.
pub fn hops_to_heart(
    progress: &[f32],
    districts: &[District],
    heart: Option<DistrictId>,
) -> Option<u16> {
    let heart = heart?;
    let mut dist: Vec<Option<u16>> = vec![None; districts.len()];
    let mut queue = VecDeque::new();
    for (index, &p) in progress.iter().enumerate() {
        if p >= 1.0 {
            dist[index] = Some(0);
            queue.push_back(index as DistrictId);
        }
    }
    while let Some(id) = queue.pop_front() {
        let next = dist[id as usize].unwrap() + 1;
        for &neighbour in &districts[id as usize].neighbours {
            if dist[neighbour as usize].is_none() {
                dist[neighbour as usize] = Some(next);
                queue.push_back(neighbour);
            }
        }
    }
    dist[heart as usize]
}

/// Новый прогон: всё в ноль, район портала осквернён с первого тика.
fn on_world_started(
    _event: On<WorldStarted>,
    districts: Res<Districts>,
    mut corruption: ResMut<Corruption>,
) {
    corruption.progress = vec![0.0; districts.len()];
    if let Some(portal) = districts.portal {
        corruption.progress[portal as usize] = 1.0;
    }
    corruption.to_heart =
        hops_to_heart(&corruption.progress, &districts.districts, districts.heart);
}

/// Шаг скверны на тике. Читает только районы, перепись и стоящие бастионы,
/// пишет только `Corruption`: с шагом движения (`move_moving_entities`,
/// порядок с `Territory` не задан) не пересекается ни по одному компоненту.
pub fn spread_corruption(
    time: Res<Time>,
    districts: Res<Districts>,
    census: Res<DistrictCensus>,
    standing: Res<BastionsStanding>,
    mut corruption: ResMut<Corruption>,
    mut commands: Commands,
) {
    if corruption.progress.len() != districts.len() {
        return;
    }
    let newly = step(
        &mut corruption.progress,
        &districts.districts,
        &census.humans,
        &standing.0,
        time.delta_secs(),
    );
    if newly.is_empty() {
        return;
    }
    corruption.to_heart =
        hops_to_heart(&corruption.progress, &districts.districts, districts.heart);
    for district in newly {
        commands.trigger(DistrictCorrupted { district });
    }
}

pub struct CorruptionPlugin;

impl Plugin for CorruptionPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Corruption>()
            .init_resource::<Corruption>()
            .add_observer(on_world_started)
            .add_systems(
                FixedUpdate,
                spread_corruption
                    .in_set(SimSet::Territory)
                    .in_set(SimPipeline::BothModes)
                    .run_if(in_state(PlayPhase::Live)),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::district_city;
    use crate::navigation::Navmesh;

    /// Цепочка из четырёх районов 0–1–2–3.
    fn chain() -> Vec<District> {
        (0..4u16)
            .map(|id| District {
                cell: IVec2::new(id as i32, 0),
                tiles: 100,
                centroid: Vec2::ZERO,
                neighbours: [id.checked_sub(1), (id < 3).then_some(id + 1)]
                    .into_iter()
                    .flatten()
                    .collect(),
                dist_to_heart: Some(3 - id),
            })
            .collect()
    }

    #[test]
    fn corruption_grows_only_next_to_the_corrupted_and_stops_at_a_bastion() {
        let districts = chain();
        let mut progress = vec![1.0, 0.0, 0.0, 0.0];
        let dt = 1.0;

        // растёт только у соседа осквернённого
        assert!(step(&mut progress, &districts, &[], &[], dt).is_empty());
        assert_eq!(progress[1], CORRUPTION_RATE);
        assert_eq!(progress[2], 0.0);
        // осквернённый не трогается
        assert_eq!(progress[0], 1.0);

        // бастион в районе 1 — стоит
        let mut held = vec![1.0, 0.0, 0.0, 0.0];
        step(&mut held, &districts, &[], &[0, 1, 0, 0], dt);
        assert_eq!(held[1], 0.0);

        // толпа замедляет: 50 человек — вдвое
        let mut crowded = vec![1.0, 0.0, 0.0, 0.0];
        step(&mut crowded, &districts, &[0, 50, 0, 0], &[], dt);
        assert!((crowded[1] - CORRUPTION_RATE / 2.0).abs() < 1e-6);

        // доходит до единицы ровно и объявляет район; со следующего шага
        // заражает дальше
        let mut progress = vec![1.0, 0.999, 0.0, 0.0];
        let newly = step(&mut progress, &districts, &[], &[], dt);
        assert_eq!(newly, vec![1]);
        assert_eq!(progress[1], 1.0);
        assert_eq!(progress[2], 0.0);
        assert!(step(&mut progress, &districts, &[], &[], dt).is_empty());
        assert!(progress[2] > 0.0);

        assert_eq!(hops_to_heart(&progress, &districts, Some(3)), Some(2));
        assert_eq!(hops_to_heart(&progress, &districts, None), None);
    }

    /// На пустой карте скверна доходит от портала до сердца за
    /// `dist_to_heart × 30 с` с точностью до тика на переход.
    #[test]
    fn an_empty_district_city_falls_in_thirty_seconds_per_hop() {
        let city = district_city();
        let mut navmesh = Navmesh::default();
        navmesh.fill_from_mapdata(&city.map);
        navmesh.prune_unreachable(navmesh.to_tile(city.portal));
        let districts = Districts::build(&navmesh, city.portal, city.heart);
        let heart = districts.heart.expect("heart district");
        let hops = districts.districts[districts.portal.unwrap() as usize]
            .dist_to_heart
            .unwrap();

        let mut progress = vec![0.0; districts.len()];
        progress[districts.portal.unwrap() as usize] = 1.0;
        let dt = 1.0 / 64.0;
        let mut ticks = 0u32;
        while progress[heart as usize] < 1.0 {
            step(&mut progress, &districts.districts, &[], &[], dt);
            ticks += 1;
            assert!(ticks < 100_000, "the heart never fell");
        }
        let expected = (hops as f32 / CORRUPTION_RATE * 64.0).round() as u32;
        assert!(
            ticks.abs_diff(expected) <= hops as u32,
            "{ticks} ticks, expected about {expected} for {hops} hops"
        );
    }
}
