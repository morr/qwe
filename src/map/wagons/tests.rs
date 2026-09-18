//! Стоящие вагоны: расстановка сцепов по вееру и слой целиком.
//!
//! Лежат здесь, а не в `wagons.rs`, как у остальных модулей шва
//! (`fences/tests.rs`, `rail/tests.rs`, `cars/tests.rs`): модуль слоя читается
//! сверху вниз как слой, а не как слой с двумя сотнями строк проверок в хвосте.

use super::*;

use crate::camera::{MAX_ZOOM, MIN_ZOOM};

/// Междупутье парка, м.
const SPACING: f32 = 5.3;

fn track(service: Option<ServiceTrack>, origin: Vec2, length: f32) -> RailLine {
    RailLine {
        points: vec![origin, origin + Vec2::new(length, 0.0)],
        width: 5.0,
        kind: RailKind::Active,
        service,
    }
}

/// Парк: `count` параллельных путей через междупутье, начиная с `origin`.
fn yard(service: Option<ServiceTrack>, origin: Vec2, length: f32, count: usize) -> Vec<RailLine> {
    (0..count)
        .map(|index| {
            track(
                service,
                origin + Vec2::new(0.0, SPACING * index as f32),
                length,
            )
        })
        .collect()
}

/// Восемь парков по пять путей, в километре друг от друга.
fn yards(service: Option<ServiceTrack>, length: f32) -> Vec<RailLine> {
    (0..8)
        .flat_map(|index| {
            yard(
                service,
                Vec2::new(100.0, 100.0 + 1000.0 * index as f32),
                length,
                5,
            )
        })
        .collect()
}

/// Сорок одиночных путей той же длины, в километре друг от друга.
fn lone(service: Option<ServiceTrack>, length: f32) -> Vec<RailLine> {
    (0..40)
        .map(|index| {
            track(
                service,
                Vec2::new(100.0, 100.0 + 1000.0 * index as f32),
                length,
            )
        })
        .collect()
}

/// В парке из служебных путей стоят составы, в пучке главных ходов — нет.
#[test]
fn wagons_stand_on_service_track_only() {
    assert!(!stable_wagons(&yards(Some(ServiceTrack::Siding), 600.0)).is_empty());
    assert!(stable_wagons(&yards(None, 600.0)).is_empty());
}

/// Станцию выдаёт веер: одиночный путь той же длины и того же класса
/// держит в разы меньше вагонов, чем путь в парке.
#[test]
fn a_lone_track_stands_almost_empty() {
    let fanned = stable_wagons(&yards(Some(ServiceTrack::Siding), 600.0)).len();
    let single = stable_wagons(&lone(Some(ServiceTrack::Siding), 600.0)).len();
    assert!(
        single * 5 < fanned,
        "{single} на одиночных против {fanned} в парках"
    );
}

/// Подъездной путь в том же парке держит меньше, чем станционный.
#[test]
fn a_spur_stands_thinner_than_a_siding() {
    let siding = stable_wagons(&yards(Some(ServiceTrack::Siding), 600.0)).len();
    let spur = stable_wagons(&yards(Some(ServiceTrack::Spur), 600.0)).len();
    assert!(
        spur < siding,
        "{spur} на подъездных против {siding} на станционных"
    );
}

/// Ширина веера — это другие пути в пределах досягаемости: свой путь не в
/// счёт, дальний сосед тоже, заброшенный путь станции не образует.
#[test]
fn the_fan_counts_other_stock_tracks_within_reach() {
    let mut rails = yard(None, Vec2::ZERO, 200.0, 3);
    rails.push(track(None, Vec2::new(0.0, 40.0), 200.0));
    let mut disused = track(None, Vec2::new(0.0, -SPACING), 200.0);
    disused.kind = RailKind::Disused;
    rails.push(disused);
    let fan = Fan::new(&rails);
    let middle = Vec2::new(100.0, SPACING);
    assert_eq!(fan.width_at(middle, 1), 2);
    assert_eq!(fan.width_at(Vec2::new(100.0, 0.0), 0), 2);
    assert_eq!(fan.width_at(Vec2::new(100.0, 40.0), 3), 0);
}

/// Заброшенный путь состава не держит.
#[test]
fn a_disused_track_stands_empty() {
    let mut rails = yards(Some(ServiceTrack::Siding), 600.0);
    for rail in &mut rails {
        rail.kind = RailKind::Disused;
    }
    assert!(stable_wagons(&rails).is_empty());
}

/// Короткий тупик — тоже: там негде.
#[test]
fn a_short_stub_stands_empty() {
    assert!(stable_wagons(&yards(Some(ServiceTrack::Siding), 30.0)).is_empty());
}

/// Тот же путь, разбитый на короткие звенья: геометрия та же, вершин больше.
fn chopped(mut rail: RailLine, links: usize) -> RailLine {
    let (start, end) = (rail.points[0], rail.points[1]);
    rail.points = (0..=links)
        .map(|index| start.lerp(end, index as f32 / links as f32))
        .collect();
    rail
}

/// Короткие звенья ломаной ничего не отнимают: сцепы идут по дуговой
/// координате **всего** пути, а не по каждому звену порознь. Посегментный
/// обход оставлял такой путь пустым целиком — каждое звено короче
/// `TRACK_MIN`.
#[test]
fn short_links_carry_the_same_rakes() {
    let straight = yards(Some(ServiceTrack::Siding), 400.0);
    let broken: Vec<RailLine> = straight
        .iter()
        .cloned()
        .map(|rail| chopped(rail, 20))
        .collect();
    let straight = stable_wagons(&straight);
    assert!(!straight.is_empty());
    assert_eq!(straight.len(), stable_wagons(&broken).len());
}

/// Вагоны идут сцепами: между соседними в сцепе — автосцепка, а не
/// произвольный зазор.
#[test]
fn wagons_come_in_rakes() {
    let wagons = stable_wagons(&yards(Some(ServiceTrack::Siding), 600.0));
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

// --- слой целиком ------------------------------------------------------
//
// Тесты на `mesh_wagons`. До шва слой собирался внутри системы Bevy: число
// вставших вагонов жило только в лог-строке, а порог зума — ранним
// возвратом у вызывающего, и ни до того, ни до другого тест не доставал.

/// Порог зума переехал в сборку: ближняя ступень рисует вагоны, дальняя
/// начинается ровно с [`WAGON_MAX_ZOOM`], и верхний край зума камеры уже за
/// ней — иначе тест на пустой слой ниже проверял бы не ту ступень.
#[test]
fn the_wagon_bucket_ends_at_its_cutoff() {
    assert_eq!(WagonZoomBucket::for_zoom(MIN_ZOOM).index, 0);
    assert_eq!(WagonZoomBucket::for_zoom(WAGON_MAX_ZOOM).index, 1);
    assert_eq!(WagonZoomBucket::for_zoom(MAX_ZOOM).index, 1);
}

/// Парк на ближней ступени даёт один слой, и он блендится: тень вагона
/// полупрозрачна, а плоский материал съел бы вершинную альфу.
#[test]
fn a_yard_builds_one_blended_layer() {
    let rails = yards(Some(ServiceTrack::Siding), 600.0);
    let (layers, report) = mesh_wagons(WagonZoomBucket::for_zoom(MIN_ZOOM), &rails);

    assert_eq!(layers.len(), 1, "тени и кузова идут одним мешем");
    assert_eq!(layers[0].name, "wagons");
    assert_eq!(layers[0].z, Z_WAGON);
    assert_eq!(layers[0].material, MaterialSpec::Blend);
    assert!(report.standing > 0);
    assert!(report.vertices > 0);
}

/// Отчёт не расходится с расстановкой: `standing` — ровно те вагоны, что
/// поставил [`stable_wagons`] на той же сцене. Это и есть число, которым
/// слой тюнился, значением вместо строки в логе.
#[test]
fn the_report_counts_the_wagons_that_stood() {
    let rails = yards(Some(ServiceTrack::Siding), 600.0);
    let (_, report) = mesh_wagons(WagonZoomBucket::for_zoom(MIN_ZOOM), &rails);

    assert_eq!(report.standing, stable_wagons(&rails).len());
}

/// Дальняя ступень: слой описан и пуст — не ранний выход у вызывающего, так
/// что деспавн в адаптере безусловен и забыть его негде.
#[test]
fn the_far_bucket_builds_an_empty_layer() {
    let rails = yards(Some(ServiceTrack::Siding), 600.0);
    let (layers, _) = mesh_wagons(WagonZoomBucket::for_zoom(MAX_ZOOM), &rails);

    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].name, "wagons");
    assert!(layers[0].builder.is_empty());
}
