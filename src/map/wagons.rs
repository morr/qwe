//! Вагоны, стоящие на станционных путях.
//!
//! Станционный парк на снимке — это не пустые нитки рельсов, а **составы**:
//! половина площади любого узла занята стоящими вагонами, и без них горловина
//! читается как схема, а не как фотография. Приём тот же, что с машинами
//! ([`super::cars`]), но слой проще: ручек стиля у него нет, и снимается он
//! только ступенью зума.
//!
//! Вагоны ставятся **только на служебные пути** (`service=siding|yard|spur`):
//! на главном ходу состав либо идёт, либо его там нет.
//!
//! Но служебный путь — ещё не станция: подъездная ветка к заводу, одиночный
//! тупик и путь парка отстоя размечены одним тегом, а вагоны на снимке стоят
//! только в последнем. Станцию выдаёт **веер**: парк — это пучок параллельных
//! путей в считаных метрах друг от друга, подъездной идёт в одиночку. Поэтому
//! каждый сцеп сперва спрашивает, сколько **других** путей проходит рядом с
//! ним ([`Fan`]), и встаёт с долей, растущей с шириной веера
//! ([`FAN_FILL`]): в парке — почти каждый, на одиночном пути — считаные.
//!
//! Ставятся **сцепами**: несколько вагонов подряд без зазора, потом пустой
//! кусок пути. Ровный ряд через равные промежутки выглядел бы как забор.
//!
//! Как и машины, вагоны — **декорация**: ни навмеша, ни симуляции.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::distance_to_segment;
use crate::map::osm::{MapData, RailKind, RailLine, ServiceTrack};
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
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
/// Сцеп — столько вагонов подряд, обе границы включительно. Счёт, а не метры,
/// поэтому целые: в `f32` верхняя граница не достигалась вовсе — `range` даёт
/// полуинтервал, и `as` усекал сцеп в 16 вагонов до 15.
const RAKE_MIN: u32 = 3;
const RAKE_MAX: u32 = 16;
/// И столько метров пустого пути после него.
const GAP_MIN: f32 = 12.0;
const GAP_MAX: f32 = 90.0;
/// Ближе этого к торцу пути не ставят: там стрелка. Отсчитывается от торцов
/// **всего** пути, а не от каждой его вершины — на изгибе стрелки нет.
const END_MARGIN: f32 = 12.0;
/// Путь короче этого сцепа не держит: за вычетом двух отступов от торцов на
/// нём остаётся меньше двух кузовов.
const TRACK_MIN: f32 = 40.0;

/// Соседний путь — тот, что проходит ближе этого к середине сцепа, м.
/// Междупутье в парке 5.3–6.5 м, так что крайний путь веера видит двух
/// соседей (5.3 и 10.6), а не одного: на 9 м он выпадал бы из собственного
/// парка.
const FAN_REACH: f32 = 12.0;
/// Доля сцепов, которые встают, по числу соседних путей: ни одного, один,
/// два, три и больше. Один сосед — это чаще всего главный ход рядом с
/// разъездом или две параллельные ветки к заводу, и станцией он ещё не
/// делает; веер начинается с двух. В полном парке встаёт чуть больше
/// половины сцепов — занятость пути выходит около 38 %, а не сплошной ряд.
///
/// Замер по кешу Тулы (служебные пути внутри карты, длина по числу соседей
/// 0/1/2/3+): разъезды 0.0/1.1/3.1/15.9 км, пути парков 1.1/2.6/5.3/8.8,
/// подъездные 5.3/3.7/2.9/3.8 — подъездной и есть «обычный» путь.
///
/// Вся строка — прежние 2/10/40/75 %, умноженные на 0.7 по отзыву автора
/// («уменьшить ещё на 30 %»): соотношение станции и одиночного пути то же,
/// меньше стало всего.
const FAN_FILL: [f32; 4] = [0.014, 0.07, 0.28, 0.525];
/// Ячейка индекса отрезков, м: отрезок записан в каждую ячейку, которой
/// касается его рамка, раздутая на [`FAN_REACH`], поэтому запросу хватает
/// одной ячейки своей точки.
const FAN_CELL: f32 = 32.0;

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
///
/// `Copy` — метку получает каждый слой модуля, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct WagonLayerTag;

/// Ступени зума: вагон втрое длиннее машины, поэтому его порог в 2.5 раза
/// дальше — свой, а не общий с [`super::cars`].
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

/// Пересборка слоя: по ступени зума и по смене солнца (у вагона своя тень).
pub fn rebuild_wagons(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<WagonZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<WagonLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let (layers, report) = mesh_wagons(*bucket, &map.rails);
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials,
        layers,
        WagonLayerTag,
    );
    info!("{report}");
}

/// Что вышло из расстановки вагонов — значением, а не только строкой в логе.
///
/// `standing` — сколько вагонов встало: число, которым этот слой тюнился
/// (1195 на Туле, потом ×0.7 до 866), и до шва его нельзя было ни на чём
/// закрепить, кроме глаза на лог-строке.
///
/// `hidden` — дальняя ступень зума: слой снят, и это состояние отчёта, а не
/// ноль в `standing`. Счётчики тогда нули, и это не заглушка — расстановка в
/// таком случае действительно не идёт, ровно как у машин
/// (`CarReport::detail = None`): снятый слой не должен стоить дороже, чем
/// стоил ранний возврат. Считать вход было бы можно только расставив вагоны,
/// то есть заплатив за то, чего никто не увидит.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WagonReport {
    pub standing: usize,
    /// Дальняя ступень зума: слой описан и пуст, расстановка не шла.
    pub hidden: bool,
    pub vertices: usize,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for WagonReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            standing,
            hidden,
            vertices,
            elapsed,
        } = self;
        if *hidden {
            return write!(f, "wagons: hidden");
        }
        write!(
            f,
            "wagons: {standing} standing ({vertices} verts) in {elapsed:?}"
        )
    }
}

/// Слой стоящих вагонов на текущей ступени зума.
///
/// **Чистая функция и единственная дверь в слой.** Дальняя ступень отдаёт
/// пустой слой, а не ранний выход у вызывающего: у вагона нет таблицы LOD, он
/// просто пропадает — 13.9-метровый кузов на 2 м/px это те же ~7 экранных
/// пикселей, на которых уже сняты машины. Говорит она об этом
/// [`WagonReport::hidden`], а не нулём в `standing`.
pub fn mesh_wagons(bucket: WagonZoomBucket, rails: &[RailLine]) -> (Vec<LayerMesh>, WagonReport) {
    let started = std::time::Instant::now();
    let hidden = bucket.index > 0;
    let wagons = if hidden {
        Vec::new()
    } else {
        stable_wagons(rails)
    };
    let builder = mesh_bodies(&wagons);
    let report = WagonReport {
        standing: wagons.len(),
        hidden,
        vertices: builder.vertex_count(),
        elapsed: started.elapsed(),
    };
    // как и у машин: тень полупрозрачна, кузов нет
    let layer = LayerMesh::new(builder, Z_WAGON, "wagons", MaterialSpec::Blend);
    (vec![layer], report)
}

/// Во сколько раз класс пути разрежает сцепы против станционного. Подъездной
/// — вдвое: даже там, где две ветки к заводу идут рядом, состав на них стоит
/// реже, чем в парке.
fn class_fill(service: ServiceTrack) -> f32 {
    match service {
        ServiceTrack::Siding | ServiceTrack::Yard => 1.0,
        ServiceTrack::Spur => 0.5,
    }
}

/// Действующие пути, но не трамвай и не заброшенные: состава они не держат,
/// а значит, и станцию собой не образуют.
fn holds_stock(rail: &RailLine) -> bool {
    rail.kind == RailKind::Active
}

/// Составы на всех служебных путях.
fn stable_wagons(rails: &[RailLine]) -> Vec<Wagon> {
    let fan = Fan::new(rails);
    let mut wagons = Vec::new();
    for (index, rail) in rails.iter().enumerate() {
        // трамвай и заброшенный путь состава не держат: по первому ходят
        // вагоны другого рода, второй разобран
        let Some(service) = rail.service.filter(|_| holds_stock(rail)) else {
            continue;
        };
        // посев пути — тот же `seed_from_point`, что у улиц и домов: три
        // перемешивающих раунда затем и нужны, что веер станционных путей идёт
        // с шагом в метры, а два раунда сводили бы соседей в один слот
        let mut rng = Lcg::new(seed_from_point(
            rail.points.first().copied().unwrap_or(Vec2::ZERO),
        ));
        let track = Track {
            rail,
            index,
            fill: class_fill(service),
        };
        stand_along(&mut wagons, &track, &fan, &mut rng);
    }
    wagons
}

/// Путь, на который ставят: сам путь, его место в `MapData::rails` (чтобы
/// веер не посчитал его собственным соседом) и множитель его класса.
struct Track<'a> {
    rail: &'a RailLine,
    index: usize,
    fill: f32,
}

/// Индекс отрезков всех путей, держащих состав, — ответ на «сколько других
/// путей проходит рядом с этой точкой».
///
/// Сетка, а не попарный проход: запросов — по одному на сцеп, отрезков — тысячи,
/// и перебор всех отрезков на каждый сцеп был бы квадратичным по городу.
struct Fan<'a> {
    rails: &'a [RailLine],
    cells: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

impl<'a> Fan<'a> {
    fn new(rails: &'a [RailLine]) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for (index, rail) in rails.iter().enumerate() {
            if !holds_stock(rail) {
                continue;
            }
            for (segment, pair) in rail.points.windows(2).enumerate() {
                let low = (pair[0].min(pair[1]) - FAN_REACH) / FAN_CELL;
                let high = (pair[0].max(pair[1]) + FAN_REACH) / FAN_CELL;
                for x in low.x.floor() as i32..=high.x.floor() as i32 {
                    for y in low.y.floor() as i32..=high.y.floor() as i32 {
                        cells.entry((x, y)).or_default().push((index, segment));
                    }
                }
            }
        }
        Self { rails, cells }
    }

    /// Сколько путей, кроме `own`, проходит ближе [`FAN_REACH`] к `point`.
    fn width_at(&self, point: Vec2, own: usize) -> usize {
        let cell = (point / FAN_CELL).floor();
        let Some(entries) = self.cells.get(&(cell.x as i32, cell.y as i32)) else {
            return 0;
        };
        let mut near: Vec<usize> = Vec::new();
        for &(index, segment) in entries {
            if index == own || near.contains(&index) {
                continue;
            }
            let points = &self.rails[index].points;
            if distance_to_segment(point, points[segment], points[segment + 1]) < FAN_REACH {
                near.push(index);
            }
        }
        near.len()
    }
}

/// Сцепы вдоль одного пути: сцеп, пустой кусок, сцеп.
///
/// Шаг идёт по дуговой координате **всего** пути ([`super::along`]), а не по
/// каждому его звену порознь: станционный путь размечен в OSM короткими
/// звеньями на кривых, и обход `points.windows(2)` выбрасывал такие звенья
/// целиком (в кеше Тулы — половину звеньев и пятую часть длины служебных
/// путей, а 11 путей из 159 оставались пусты только из-за него), отступал от
/// каждой внутренней вершины, где никакой стрелки нет, и сбрасывал на ней фазу
/// сцепа. Тот же обход и по той же причине оставил слой машин
/// ([`super::cars::park_along`]).
///
/// Каждый сцеп сперва бросает кость против доли своего места: класс пути ×
/// [`FAN_FILL`] по ширине веера у середины сцепа. Невставший сцеп оставляет
/// пустым тот кусок пути, который занял бы, — пустоты на одиночном пути от
/// этого длинные, а фаза сцепов вдоль пути не зависит от того, какие встали.
fn stand_along(wagons: &mut Vec<Wagon>, track: &Track, fan: &Fan, rng: &mut Lcg) {
    let rail = track.rail;
    let (along, total) = arclengths(&rail.points);
    if total < TRACK_MIN {
        return;
    }
    let pitch = WAGON_LENGTH + COUPLED_GAP;
    let mut at = END_MARGIN + rng.range(0.0, GAP_MAX);
    // последний **поставленный** вагон: у кузова жёсткая база 13.9 м, и на
    // изломе пути соседи по сцепу наезжают друг на друга. Проверка по мировому
    // расстоянию, а не по дуговой координате, — она ловит любую кривизну
    let mut last: Option<Vec2> = None;
    while at + WAGON_LENGTH <= total - END_MARGIN {
        let rake = rng.range(RAKE_MIN as f32, RAKE_MAX as f32 + 1.0) as u32;
        let span = rake as f32 * pitch;
        let middle = (at + span / 2.0).min(total - END_MARGIN);
        let width = place_on_path(&rail.points, &along, middle)
            .map_or(0, |(point, _)| fan.width_at(point, track.index));
        let fill = track.fill * FAN_FILL[width.min(FAN_FILL.len() - 1)];
        if rng.next_f32() >= fill {
            at += span + rng.range(GAP_MIN, GAP_MAX);
            continue;
        }
        for _ in 0..rake {
            if at + WAGON_LENGTH > total - END_MARGIN {
                break;
            }
            let centre = at + WAGON_LENGTH / 2.0;
            at += pitch;
            let Some((point, direction)) = place_on_path(&rail.points, &along, centre) else {
                continue;
            };
            if last.is_some_and(|previous| previous.distance(point) < WAGON_LENGTH) {
                continue;
            }
            last = Some(point);
            wagons.push(Wagon {
                at: point,
                along: direction,
                color: WAGON_COLORS
                    [(rng.next_f32() * WAGON_COLORS.len() as f32) as usize % WAGON_COLORS.len()],
            });
        }
        at += rng.range(GAP_MIN, GAP_MAX);
    }
}

/// Все тени, потом все кузова: иначе тень вагона легла бы на соседний.
///
/// Только меш, без слоя — имя `mesh_wagons` ушло функции слоя, как у машин
/// (`cars::mesh_bodies` под `cars::mesh_cars`).
fn mesh_bodies(wagons: &[Wagon]) -> MeshBuilder {
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

#[cfg(test)]
mod tests {
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
    fn yard(
        service: Option<ServiceTrack>,
        origin: Vec2,
        length: f32,
        count: usize,
    ) -> Vec<RailLine> {
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
}
