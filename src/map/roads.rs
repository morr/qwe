//! Слой дорог, аллей и стен Кремля: по ленте на `RoadLine`/`WallLine`, слитой
//! в merged-меш на класс. Тумблеры слоёв — ресурс [`RoadStyle`], форма —
//! [`RoadShape`] (`roads/shape.rs`); оба переключаются на лету панелью
//! (`ui/roads.rs`, `ui/road_paint.rs`), и правка пересобирает только эти слои
//! ([`rebuild_roads`]). Рельсовые пути — в `map/rail.rs`, трамвай — в
//! `map/tram.rs`: у обоих свой стиль и свой зум-LOD, и пересобираются они по
//! зуму, а не по [`RoadStyle`].
//!
//! Мост (`RoadLine::bridge`) уходит из слоёв своего класса в тройку
//! `bridge_shadows` + `bridge_casings` + `bridges`: серый бордюр по краям
//! настила и заливка цветом
//! класса над `Z_ROAD` — эстакада кроет улицу, которую пересекает, а ровные
//! торцы бордюра читаются как края настила, вид 2ГИС. Под ними —
//! [`bridges::push_bridge_shadows`]: единственное на карте, что говорит, что
//! настил поднят, потому что наземные тени считают только дома. Всё мостовое
//! — цепочки ways, бордюр, тень — в `roads/bridges.rs`.
//!
//! Раньше дороги рисовал `MeshBuilder::push_polyline` — свой квад на
//! сегмент, продлённый с обоих концов на полуширины. Стыков у него нет вообще:
//! на изломе продление торчит за внешний угол прямоугольным выступом, между
//! двумя выступами остаётся выемка, а торец пути — квадратный шип. Лента
//! теперь всегда [`RoadJoin::Round`] ([`ROAD_JOIN`]) — дуга на внешней стороне
//! излома и полудиск на
//! торце, то же самое, что `stroke-linejoin: round` + `stroke-linecap: round`
//! у Mapnik, которым нарисован osm-carto: круглые торцы двух ways в общем узле
//! перекрываются и сливаются в скруглённый стык.
//!
//! Осевая (`RoadLine::points`) при этом **не трогается**: на ней стоят навмеш
//! (`bridge`/`passage`-прорезы), арки, посадка деревьев и генератор дверей.
//! Chaikin-сглаживание работает на копии и только ради картинки; само правило
//! живёт в `map/smooth.rs` — его читают ещё пять слоёв, — а здесь остаётся
//! [`centerline`], дорожная обёртка над ним с её двумя закреплениями.
//!
//! Улица — это не одна лента, а три слоя: **тротуар** (`Z_SIDEWALK`, светлая
//! полоса шире проезжей части на полосу [`RoadLine::sidewalk`] с каждой стороны), кант и
//! заливка асфальтом. Тротуар лежит под лентами **улицы** по той же логике, что
//! кант: заливка поперечной улицы кроет его на перекрёстке, и тротуар
//! обрывается там, где обрывается в жизни, — но **поверх ленты аллеи**: дорожка,
//! выходящая на улицу, упирается в тротуар, как упирается в бордюр на
//! фотографии, вместо того чтобы перечеркнуть полосу песочной лентой. **Разметка** — линии по границам
//! полос ([`lane_count`]) — отдельный **слой краски** (`roads/paint.rs`):
//! геометрия от оси улицы на той же раскладке полос, по которой шейдер
//! асфальта кладёт колею. Линия **рвётся на перекрёстках** — по общим узлам
//! ways (`roads/junctions.rs`), а не по торцам, так что way, разрезанный посреди
//! квартала, несёт линию сквозь стык, а сквозная улица теряет её ровно на
//! ширину поперечной. Широкие улицы кладутся поверх узких: заливка магистрали
//! кроет торец жилой улицы, и колея въезда гаснет под ней.

use std::borrow::Cow;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

pub use self::bridges::BridgeReport;
use self::bridges::Bridges;
pub use self::drawn::{Axis, Drawn, DrawnStats};
use self::network::RoadNodes;
pub use self::node_paint::CrossingMode;
use self::shape::{RoadShape, RoadShapeOnMap};
use crate::map::SunOnMap;
use crate::map::footprint::JOIN_EPSILON;
use crate::map::meshing::{
    Break, LaneFrame, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin, RibbonShape, miter_offsets,
    to_break_beyond,
};
use crate::map::osm::model::{RoadNodeKind, point_in_area, polyline_length, ring_bounds};
use crate::map::osm::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
use crate::map::shapes::{is_ring, push_shape};
use crate::map::smooth::{Smoothing, smooth_pinned};
use crate::map::spawn::GRASS_COLOR;
use crate::map::surface::{
    self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, SurfaceKind, spawn_layers,
};
use crate::prefs::retuned;
use crate::settings::{
    Z_ALLEY, Z_BUILDING, Z_LOT_LINES, Z_LOT_SIDEWALK, Z_ROAD, Z_ROAD_MEDIAN, Z_SIDEWALK,
};

/// Проезжая часть — асфальт: серый, заметно темнее тротуара и земли. Белой
/// (osm-carto) она была, пока не появилась разметка: белую линию на белом не
/// видно. Потом была светло-голубовато-серой (0.655, как на детальных картах
/// 2ГИС), и это картографический тон, а не снимок: у выветренного асфальта на
/// аэрофото нейтральный серый около середины шкалы, и ступень до светлого
/// бетонного тротуара там заметно больше. Тот же тон у стоянок
/// (`spawn::PARKING_COLOR`) — они лежат поверх улиц одним полотном с ними.
/// Открыт наружу витрине машин: ряд обязан стоять на том же асфальте, что в
/// городе, — на своём сером ступень яркости между кузовом и покрытием была бы
/// не та.
pub const ROAD_COLOR: Color = Color::srgb(0.545, 0.545, 0.55);
/// Асфальт над трамвайными путями (`roads/tram_band.rs`): светлее
/// [`ROAD_COLOR`] едва заметно, как на Яндексе, — трамвайная полоса читается
/// цветом, а не разметкой.
pub const TRAM_BAND_COLOR: Color = Color::srgb(0.59, 0.59, 0.595);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Белая разметка на асфальте стоянки: двойная сплошная между встречными
/// полотнами (`roads/lots.rs`), слой `lot_lines`. Обводка со штриховкой
/// направляющего островка (`roads/gores.rs`) — не здесь: она в слое краски
/// `road_paint_islands`.
const LOT_LINE_COLOR: Color = Color::srgb(0.88, 0.88, 0.86);

/// Тротуар — светлый бетон между асфальтом и тёплой землёй: светлее проезжей
/// части на четверть, и именно эта ступень яркости читается как бордюр.
const SIDEWALK_COLOR: Color = Color::srgb(0.82, 0.815, 0.80);
/// Кусок тротуара короче этого, м, не кладётся ([`push_sidewalk`]): между
/// кусками пары остаются обрезки в сантиметры.
const SIDEWALK_PIECE_MIN: f32 = 0.5;
/// Самый длинный кусок поперечной улицы между половинами одной пары, м: две
/// половины и самый широкий газон между ними. Такой кусок лежит в проёме
/// разделительной, и тротуара у него нет.
const MEDIAN_CROSSING_MAX: f32 = 40.0;
/// На сколько асфальт кармана стоянки заходит под кромку ленты, м: встык
/// между ними светилась бы щель.
const POCKET_OVERLAP: f32 = 0.05;

/// Полоса не у́же этого, м. У разобранной улицы ширина выведена из самих
/// полос (`network::sections`), и зажим ничего не режет; он остался для
/// дорог, собранных руками, где `lanes=6` может прийти на десятиметровую
/// ленту.
const MIN_LANE_WIDTH: f32 = 2.5;
/// Полос по умолчанию, когда тега `lanes` нет: двусторонней улице — по паре
/// на каждые 7 м ширины (8 и 10 м — две полосы, 12 и 16 — четыре),
/// односторонней — по полосе на 4.5 м (8 м — одна, без линий; 16 — три).
const TWOWAY_METERS_PER_LANE_PAIR: f32 = 7.0;
const ONEWAY_METERS_PER_LANE: f32 = 4.5;

/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

/// Чем закрыт излом ленты дороги.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadJoin {
    /// Статус-кво до сглаживания: свой квад на сегмент, оба конца продлены на
    /// полуширины. Оставлен, чтобы можно было сравнить с прежней картинкой.
    Square,
    /// Сведение по биссектрисе с ограничением длины стыка.
    Miter,
    /// Дуга на внешней стороне излома + полудиск на торце — вид osm-carto.
    #[default]
    Round,
}

impl RoadJoin {
    pub const ALL: [Self; 3] = [Self::Square, Self::Miter, Self::Round];

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Miter => "Miter",
            Self::Round => "Round",
        }
    }

    /// Излом и торец ленты `MeshBuilder`. `None` — `Square`: ленты у него нет
    /// вовсе, это `push_polyline` с продлёнными торцами. Одна таблица на всех,
    /// кто кладёт ленту дороги ([`push_ribbon`], [`ROAD_RIBBON`]) —
    /// разойдясь, они дали бы двум слоям одной улицы разные торцы.
    const fn ribbon_shape(self) -> Option<(RibbonJoin, RibbonCap)> {
        match self {
            Self::Square => None,
            Self::Miter => Some((RibbonJoin::Miter, RibbonCap::Butt)),
            Self::Round => Some((RibbonJoin::Round, RibbonCap::Round)),
        }
    }
}

/// Стык и торец дорожной ленты. Выбора больше нет: углы и стыки узлов строит
/// полигон узла (`roads/corners.rs`), и `Square` держали только для сравнения
/// со старой картинкой. Сам [`RoadJoin`] остаётся ручкой полосы посадки
/// аллей (`TreeRowStyle`).
pub const ROAD_JOIN: RoadJoin = RoadJoin::Round;

/// [`ROAD_JOIN`] как излом и торец ленты `MeshBuilder`, развёрнутый на
/// компиляции: у дорожной ленты стык всегда лента, и ветки «`Square` — это
/// `push_polyline`» у заливки проезжей части нет.
const ROAD_RIBBON: (RibbonJoin, RibbonCap) = match ROAD_JOIN.ribbon_shape() {
    Some(shape) => shape,
    None => panic!("ROAD_JOIN is not a ribbon join"),
};

/// Тумблеры дорожных слоёв; переключаются панелью (секции Roads и Road paint)
/// и BRP, сохраняются в настройках между запусками. Правка пересобирает
/// дорожные слои ([`rebuild_roads`]).
///
/// Прежние `join`, `smoothing` и `casing` ушли: стык строит полигон узла,
/// сглаживание стало допуском в метрах ([`RoadShape::curve_tolerance`]), а
/// тёмный кант — картографический приём, который с бордюром и тротуаром не
/// нужен. Их ключи в `settings.toml` с другой ветки читаются и молча
/// пропускаются: `bevy_settings` применяет только поля, которые у типа есть.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roads")]
pub struct RoadStyle {
    /// Серая полоса тротуара вдоль улиц (не проездов) отдельным слоем под
    /// всеми лентами.
    pub sidewalks: bool,
    /// Разметка полос на проезжей части улиц — линия на каждой границе полос,
    /// с разрывами на перекрёстках; слой краски (`roads/paint.rs`). Колея
    /// асфальта от неё не зависит — у неё своя ручка (`RoadPaintStyle`).
    pub markings: bool,
    /// Зебры на плечах узлов и на переходах (`roads/node_paint.rs`).
    pub crossings: CrossingMode,
    /// Стоп-линии на плечах, что уступают, и у регулируемых переходов.
    pub stop_lines: bool,
    /// Стрелки на полосах подходов к узлу (`roads/turns.rs`).
    pub arrows: bool,
}

impl Default for RoadStyle {
    fn default() -> Self {
        Self {
            sidewalks: true,
            markings: true,
            crossings: CrossingMode::default(),
            stop_lines: true,
            arrows: false,
        }
    }
}

/// [`RoadLine::is_carriageway`] свободной функцией — её зовут как предикат
/// (`filter(is_carriageway)`) по всему `map/`.
///
/// Открыт наружу для [`map::cars`](crate::map::cars): «улица, вдоль которой
/// паркуются» — то же самое понятие, что «улица, у которой есть тротуар и
/// разметка», и второй копии предиката у слоя машин быть не должно.
pub fn is_carriageway(road: &RoadLine) -> bool {
    road.is_carriageway()
}

/// Число полос проезжей части: из сечения ([`RoadLine::lanes`] — после
/// разбора оно есть у каждой улицы, `network::sections`), у дороги без него
/// (собранной тестом руками) — дефолт по ширине, и не больше, чем влезает по
/// [`MIN_LANE_WIDTH`]. Кольцо — как любая улица: разрывы на въездах даёт
/// краска узла, и линия двухполосного кольца идёт между ними.
pub fn lane_count(road: &RoadLine) -> u8 {
    let most = ((road.width / MIN_LANE_WIDTH).floor() as u8).max(1);
    let lanes = match road.lanes {
        Some(lanes) => lanes,
        None if road.oneway => (road.width / ONEWAY_METERS_PER_LANE).floor() as u8,
        None => 2 * (road.width / TWOWAY_METERS_PER_LANE_PAIR).round() as u8,
    };
    lanes.clamp(1, most)
}

/// Плавный переход 0 → 1 по доле `t` (smoothstep, `t` зажато в 0..1): одна
/// кривая на все переходы разметки и полотна — разводку половин
/// (`network/pairs.rs`), кромку слияния (`merges.rs`) и раскладку полос на его
/// клине (`paint.rs::MergeRamp`), которые обязаны идти друг по другу.
pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Граней у круга разворотной площадки.
const TURNING_CIRCLE_SIDES: usize = 32;
/// Радиус разворотной площадки по отношению к полуширине дороги и его
/// пределы, м: легковой машине на развороте нужно метров шесть, мусоровозу —
/// десять, а площадка шире дороги, которая к ней ведёт, раза в два.
const TURNING_CIRCLE_SCALE: f32 = 2.2;
const TURNING_CIRCLE_RADIUS: std::ops::RangeInclusive<f32> = 6.0..=10.0;

/// Радиус разворотной площадки в тупике дороги шириной `width`, м.
fn turning_radius(width: f32) -> f32 {
    (width / 2.0 * TURNING_CIRCLE_SCALE)
        .clamp(*TURNING_CIRCLE_RADIUS.start(), *TURNING_CIRCLE_RADIUS.end())
}

/// Дуги колец ([`rings`]) сечением всего кольца: ширина и полосы —
/// наибольшие по его дугам. У дуг одного кольца в OSM бывает разное `lanes`
/// (3 и 2 на кольце primary в Туле), и лента шла бы ступенями.
fn ring_arcs(roads: &[RoadLine], rings: &rings::Rings) -> Vec<(usize, RoadLine)> {
    rings
        .list
        .iter()
        .flat_map(|ring| {
            let width = ring
                .roads
                .iter()
                .map(|&road| roads[road].width)
                .fold(0.0, f32::max);
            let lanes = ring
                .roads
                .iter()
                .filter_map(|&road| roads[road].lanes)
                .max();
            ring.roads
                .iter()
                .filter(move |&&road| roads[road].width != width || roads[road].lanes != lanes)
                .map(move |&road| {
                    let arc = RoadLine {
                        width,
                        lanes,
                        ..roads[road].clone()
                    };
                    (road, arc)
                })
        })
        .collect()
}

/// Тротуар кольца и бордюр его острова. Тротуар — только снаружи, лентой по
/// всему кольцу сразу, без швов между дугами; внутри вместо тротуарного
/// кольца — бордюр [`medians::MEDIAN_KERB`] по кромке острова, как у газона
/// разделительной.
fn push_ring_edges(
    builder: &mut MeshBuilder,
    ring: &rings::Ring,
    [width, sidewalk]: [f32; 2],
    color: LinearRgba,
) {
    let path = &ring.path[..ring.path.len() - 1];
    if path.len() < 3 {
        return;
    }
    // сдвиг от оси: плюс — наружу
    let shifted = |shift: f32| -> Vec<Vec2> {
        let outward = if ring.ccw { -shift } else { shift };
        path.iter()
            .zip(miter_offsets(path, true, outward))
            .map(|(point, offset)| *point + offset)
            .collect()
    };
    if sidewalk > 0.0 {
        builder.push_ribbon(
            &shifted(sidewalk / 2.0),
            true,
            width + sidewalk,
            color,
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }
    let kerb = medians::MEDIAN_KERB;
    builder.push_ribbon(
        &shifted(-(width + kerb) / 2.0),
        true,
        kerb,
        color,
        RibbonJoin::Miter,
        RibbonCap::Butt,
    );
}

/// Раскладка полос проезжей части ([`paint::lane_frame`]) — одна на колею
/// асфальта и на линии слоя краски. У проезда и дорожки полос нет, и колеи
/// тоже; однополосная улица колею получает, а линий у неё нет.
fn road_lanes(road: &RoadLine) -> Option<LaneFrame> {
    is_carriageway(road).then(|| paint::lane_frame(lane_count(road)))
}

/// Порядок заливки лент: узкие под широкими, ведущие узлов — поверх всех
/// (см. доку модуля), и **в каждом узле его плечи — до его ведущей**. Одного
/// «ведущие последними» мало: примыкание, что само ведёт другой узел дальше,
/// попадало в хвост вместе с главной и, если было шире, ложилось на неё
/// торцом — квадрат без колеи посреди перекрёстка (Тула, 5968, 1582).
/// Топологическая сортировка с приоритетом по прежнему ключу; на цикле
/// (две дороги ведут узлы друг друга) берётся первая по ключу.
fn fill_order(widths: &[f32], leading: &[bool], junctions: &[node_paint::Junction]) -> Vec<usize> {
    use std::collections::BTreeSet;
    let mut by_key: Vec<usize> = (0..widths.len()).collect();
    by_key.sort_by(|&a, &b| {
        leading[a]
            .cmp(&leading[b])
            .then(widths[a].total_cmp(&widths[b]))
    });
    let mut rank = vec![0; widths.len()];
    for (place, &road) in by_key.iter().enumerate() {
        rank[road] = place;
    }
    // ребро «плечо → ведущая»: плечо ложится раньше
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); widths.len()];
    let mut before = vec![0usize; widths.len()];
    for junction in junctions {
        let mut arms: Vec<usize> = junction.arms.iter().map(|arm| arm.road).collect();
        arms.sort_unstable();
        arms.dedup();
        for &lead in &junction.leading {
            for &arm in arms.iter().filter(|&&arm| !junction.leading.contains(&arm)) {
                if !after[arm].contains(&lead) {
                    after[arm].push(lead);
                    before[lead] += 1;
                }
            }
        }
    }
    let mut ready: BTreeSet<usize> = (0..widths.len())
        .filter(|&road| before[road] == 0)
        .map(|road| rank[road])
        .collect();
    let mut left: BTreeSet<usize> = (0..widths.len()).map(|road| rank[road]).collect();
    let mut order = Vec::with_capacity(widths.len());
    while let Some(&first) = left.first() {
        let place = ready.pop_first().unwrap_or(first);
        ready.remove(&place);
        left.remove(&place);
        let road = by_key[place];
        order.push(road);
        for &next in &after[road] {
            before[next] = before[next].saturating_sub(1);
            if before[next] == 0 && left.contains(&rank[next]) {
                ready.insert(rank[next]);
            }
        }
    }
    order
}

/// Шаг, с которым осевая крепостной стены проверяется на «стоит ли тут
/// здание стены», м.
const WALL_PROBE_STEP: f32 = 2.0;

/// Крепостные сооружения карты (`AreaKind::Kremlin`) с их AABB — чтобы лента
/// `barrier=city_wall` не рисовалась поверх стены, которая уже нарисована
/// зданием.
///
/// Лента — единственный рисунок стены там, где мапер провёл только линию. Но
/// у Тульского кремля есть и `building=wall`, и башни, и красная лента
/// ложилась по ним сверху: в 2.5D — тёмно-оранжевой обводкой рядом с поднятой
/// стеной, у башен — кругами поверх шатров. Поэтому лента режется на куски, и
/// рисуются только те, что идут **мимо** крепостных зданий. Навмеша это не
/// касается: он по-прежнему блокирует всю ленту.
struct Fortresses<'a> {
    areas: Vec<(&'a PolyArea, (Vec2, Vec2))>,
}

impl<'a> Fortresses<'a> {
    fn of(buildings: &'a [PolyArea]) -> Self {
        Self {
            areas: buildings
                .iter()
                .filter(|building| building.kind == AreaKind::Kremlin)
                .map(|building| (building, ring_bounds(&building.outer)))
                .collect(),
        }
    }

    fn covers(&self, point: Vec2) -> bool {
        self.areas.iter().any(|(area, (min, max))| {
            point.cmpge(*min).all() && point.cmple(*max).all() && point_in_area(point, area)
        })
    }

    /// Куски осевой, не накрытые крепостными зданиями. Каждый отрезок
    /// проверяется точками через [`WALL_PROBE_STEP`]; кусок начинается и
    /// кончается на такой точке.
    fn bare_runs(&self, points: &[Vec2]) -> Vec<Vec<Vec2>> {
        if self.areas.is_empty() {
            return vec![points.to_vec()];
        }
        let mut runs = Vec::new();
        let mut current: Vec<Vec2> = Vec::new();
        let mut cut = false;
        let mut visit = |point: Vec2, runs: &mut Vec<Vec<Vec2>>| {
            if self.covers(point) {
                cut = true;
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                }
                current.clear();
            } else {
                current.push(point);
            }
        };
        for (index, pair) in points.windows(2).enumerate() {
            let steps = (pair[0].distance(pair[1]) / WALL_PROBE_STEP)
                .ceil()
                .max(1.0) as usize;
            // первая точка отрезка — только у первого, дальше она уже была
            // концом предыдущего
            let from = usize::from(index > 0);
            for step in from..=steps {
                visit(pair[0].lerp(pair[1], step as f32 / steps as f32), &mut runs);
            }
        }
        if current.len() >= 2 {
            runs.push(current);
        }
        // Огрызок между двумя зданиями — не стена, а зазор разметки: осевая
        // `city_wall` и контур башни в OSM расходятся на метр-другой, и на
        // Тульском кремле у угловой башни от ленты оставался красный язычок.
        // Режется только там, где лента вообще резалась: неразрезанная линия
        // любой длины — единственный рисунок своей стены.
        if cut {
            runs.retain(|run| polyline_length(run) >= WALL_STUB_MAX);
        }
        runs
    }
}

/// Кусок крепостной ленты короче этого между крепостными зданиями не рисуется, м.
/// Двенадцати не хватило: у северо-восточных башен Тульского кремля осевая
/// расходится с контурами на куски в пятнадцать–тридцать метров, и от ленты
/// оставались красные крюки у каждого угла. Прясло стены между башнями длиннее
/// сорока метров, так что настоящий неразмеченный кусок стены под порог не
/// попадает.
const WALL_STUB_MAX: f32 = 40.0;

/// Дорожный слой карты — чтобы пересборка стиля знала, что деспавнить.
///
/// `Copy` — метку получает каждый из девяти слоёв, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct RoadLayerTag;

/// Что вышло из сборки дорожных слоёв — значением, а не только строкой в логе.
///
/// Здесь живут числа, которыми тюнилась вся эта область и которые до шва
/// нельзя было ни на чём закрепить: 8710 закруглений кербов на Туле, 903 из
/// них в полосе тротуара, 39 стежков, 8 переездов. `network` — сколько из
/// общего времени ушло **до первой ленты**, то есть на сеть, стежки и углы.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RoadReport {
    /// Стиль, которым всё это нарисовано: тумблеры из лог-строки — это он.
    pub style: RoadStyle,
    pub junctions: usize,
    /// Куски линий слоя краски (`roads/paint.rs`) и их вершины — отдельно от
    /// общего счёта: краска строится своими мешами и прячется с зумом.
    pub paint_lines: usize,
    pub paint_vertices: usize,
    /// Краска узлов (`roads/node_paint.rs`): зебры (из них по OSM),
    /// стоп-линии, карманы, кластеры сближенных узлов и проходы главной
    /// сквозь узел.
    pub zebras: [usize; 2],
    pub stop_lines: usize,
    pub pockets: usize,
    pub clusters: usize,
    /// Карманы стоянки вдоль улиц (`roads/pockets.rs`) и разворотные
    /// площадки в тупиках.
    pub kerb_pockets: usize,
    pub turning_circles: usize,
    pub through: usize,
    /// Траектории узлов (`roads/turns.rs`) — кривые манёвров; дороги, ведущие
    /// хоть один узел (колея сквозь).
    pub turns: usize,
    pub leading: usize,
    /// Стрелки на полосах подходов (`roads/turns.rs`).
    pub arrows: usize,
    pub kerb_returns: usize,
    pub sidewalk_returns: usize,
    /// Наружные углы узлов (`roads/corners.rs`): асфальт и тротуар.
    pub outer_corners: [usize; 2],
    /// Подготовка дорог (`roads/drawn.rs`): переезды, стежки, клинья,
    /// слияния, разделительные, кольца, швы осей — одним значением.
    pub drawn: DrawnStats,
    /// Мосты (`roads/bridges.rs`): мостовых ways, мостов-цепочек из них и
    /// мостов с тенью.
    pub bridges: BridgeReport,
    /// Острова-крошки в треугольниках узлов, залитые асфальтом
    /// (`corners::small_islands`).
    pub islands: usize,
    /// Направляющие островки у колец (`roads/gores.rs`).
    pub gores: usize,
    /// Из данных v15 (`roads/islands.rs`): островков-точек на улицах, контуров
    /// островков и контуров полотна.
    pub road_islands: [usize; 3],
    /// Кромки, сведённые на слияниях (`drawn.merges`) к кромке продолжения
    /// (`roads/merges.rs`): их кладёт лента, не подготовка.
    pub merge_edges: usize,
    /// Куски светлой полосы над трамвайными путями (`roads/tram_band.rs`).
    pub tram_bands: usize,
    pub vertices: usize,
    pub network: std::time::Duration,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for RoadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            style,
            junctions,
            paint_lines,
            paint_vertices,
            zebras: [zebras, osm_zebras],
            stop_lines,
            pockets,
            clusters,
            kerb_pockets,
            turning_circles,
            through,
            turns,
            leading,
            arrows,
            kerb_returns,
            sidewalk_returns,
            outer_corners: [outer, outer_sidewalks],
            drawn:
                DrawnStats {
                    crossings,
                    stitches,
                    seams,
                    tight,
                    tapers,
                    merges,
                    medians: [paved, lawns, beds],
                    rings: [rings, webs],
                },
            bridges:
                BridgeReport {
                    ways: bridge_ways,
                    bridges,
                    casting,
                },
            islands,
            gores,
            road_islands: [refuges, island_areas, carriageways],
            merge_edges,
            tram_bands,
            vertices,
            network,
            elapsed,
        } = self;
        write!(
            f,
            "road meshing: {vertices} verts in {elapsed:?} (sidewalks {}, markings {}, paint {paint_lines} lines / {paint_vertices} verts, \
             junctions {junctions} ({clusters} clusters, main through {through}), zebras \
             {zebras} ({osm_zebras} from OSM), stop lines {stop_lines}, pockets {pockets}, \
             turn paths {turns}, arrows {arrows}, leading roads {leading}, kerb returns {kerb_returns} + \
             {sidewalk_returns} on sidewalks, outer corners {outer} + {outer_sidewalks} on \
             sidewalks, stitches {stitches}, kerb pockets {kerb_pockets}, turning circles {turning_circles}, driveway crossings \
             {crossings}, rings {rings} ({webs} webs), small islands {islands}, gores {gores}, safety islands {refuges} + {island_areas} areas, \
             carriageway areas {carriageways}, tapers {tapers}, merges {merges} ({merge_edges} edges), medians {paved} paved + {lawns} \
             lawn (tram beds {beds}), tram bands {tram_bands}, smooth seams {seams}, tight corners {tight}, bridges {bridges} of \
             {bridge_ways} ways ({casting} cast shadows); {network:?} of it before the \
             ribbons)",
            style.sidewalks, style.markings,
        )
    }
}

/// Восемнадцать дорожных слоёв в выбранном стиле и форме: заливка аллей,
/// тротуары, газон разделительных, заливка улиц, два слоя большой стоянки,
/// три мостовых слоя, лента крепостной стены и восемь слоёв краски
/// (`roads/paint.rs`) над своим асфальтом.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_roads`] и `spawn_map`), и тест. Это самый
/// крупный модуль шва, и он же самый показательный: восемнадцать слоёв, четыре
/// вида материала и вся телеметрия области — всё уезжает через один возврат.
pub fn mesh_roads(
    map: &MapData,
    style: RoadStyle,
    shape: RoadShape,
) -> (Vec<LayerMesh>, RoadReport) {
    let started = std::time::Instant::now();
    let (roads, walls): (&[RoadLine], &[WallLine]) = (&map.roads, &map.walls);
    let mut painter = paint::Painter::new(map.traffic_side);

    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut alleys = MeshBuilder::with_surface_coords();
    let mut streets = MeshBuilder::with_surface_coords();
    // мост — цепочка ways, и тень считается по всей цепочке; мостовые слои
    // копит он же (`roads/bridges.rs`)
    let mut bridges = Bridges::new(map);
    let mut wall_ribbons = MeshBuilder::default();
    // улицы на больших стоянках — бордюром и разметкой поверх их асфальта
    // (`roads/lots.rs`)
    let mut grounds = lots::Grounds::of(map);

    // Дороги так, как они рисуются (`roads/drawn.rs`): переезд через тротуар —
    // асфальтом проезда, а не песочной дорожкой, дуга кольца — сечением всего
    // кольца; узлы, оси улиц, стежки, клинья и слияния — там же.
    let prepared = Drawn::new(map, &style, &shape);
    let nodes = prepared.nodes();
    let drawn = prepared.roads();
    // узловые оси — скругления, слияния, карманы, станции штрихов, полотно
    // трамвая: их торцы стоят в точках OSM
    let nodal = prepared.axes(Axis::Nodal);
    // длина улицы у начала каждого way — по ней идут штрихи краски
    let stations = paint::street_stations(&map.network, &nodal);
    // Скругления кладутся раньше всех лент своего слоя: лента поверх кроет
    // скругление, а не наоборот, и разметка остаётся целой.
    let kerb_returns = corners::kerb_returns(&prepared, shape.corner_radius());
    let islands = corners::small_islands(&prepared);
    // Узлы (`roads/junctions.rs`), стежки среди них: базовые разрывы
    // асфальта, краска узлов, разрывы ряда у бордюра — одним значением. По
    // ним рвётся краска и гаснет колея асфальта — колея есть и с выключенной
    // разметкой, так что считаются они всегда. Краска строится и без
    // разметки: ведущая дорога узла и плечи для траекторий — это колея
    // асфальта, а не краска
    let mut junctions = junctions::Junctions::new(
        &prepared,
        map,
        &islands,
        node_paint::NodePaintStyle {
            crossings: if style.markings {
                style.crossings
            } else {
                CrossingMode::Off
            },
            stop_lines: style.markings && style.stop_lines,
        },
    );
    for (class, outline) in &kerb_returns.roads {
        let (builder, color) = match class {
            RoadClass::Street => (&mut streets, ROAD_COLOR),
            RoadClass::Alley => (&mut alleys, ALLEY_COLOR),
        };
        // Скругление не выпукло, но веер из его первой вершины — угла краёв —
        // верен: дуга между точками касания и есть та часть окружности, что
        // видна из угла. `earcutr` на восьми тысячах таких фигур стоил бы
        // больше самой укладки.
        builder.push_convex(outline, color.to_linear());
    }
    // и тот же угол в слое тротуаров: полоса поворачивает за бордюром
    for outline in &kerb_returns.sidewalks {
        sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
    }
    // кромки половин, сходящиеся к кромкам продолжения, — тоже до лент
    let mut merge_edges = 0;
    for merge in &prepared.merges().list {
        let bands = merges::merge_bands(
            merge,
            &drawn,
            &nodal,
            &map.network,
            |road| prepared.sidewalk_drawn(road),
            shape.taper(),
        );
        for band in bands {
            streets.push_polygon(&band.asphalt, &[], ROAD_COLOR.to_linear());
            if let Some(outline) = &band.sidewalk {
                sidewalks.push_polygon(outline, &[], SIDEWALK_COLOR.to_linear());
            }
            merge_edges += 1;
        }
    }
    // карманы — по тому же ответу и тем же разрывам, что ряд машин
    // (`map::cars`): асфальт за кромкой и тротуар, отодвинутый за него
    let kerbsides = pockets::all_kerbsides(
        roads,
        nodes,
        &nodal,
        junctions.row(),
        map.traffic_side,
        prepared.lots(),
    );
    let mut kerb_pockets = 0;
    for (index, road) in roads.iter().enumerate() {
        let half = road.width / 2.0;
        let sidewalk = prepared.sidewalk_drawn(index);
        for kerbside in &kerbsides[index] {
            let sidewalk =
                sidewalk.filter(|_| road.sidewalk().sides()[usize::from(kerbside.side < 0.0)]);
            for pocket in &kerbside.pockets {
                let outline = |outer: f32| {
                    pockets::outline(
                        &nodal[index],
                        pocket,
                        kerbside.side,
                        [half - POCKET_OVERLAP, outer],
                    )
                };
                let edge = half + pockets::POCKET_WIDTH;
                streets.push_polygon(&outline(edge), &[], ROAD_COLOR.to_linear());
                if let Some(sidewalk) = sidewalk {
                    sidewalks.push_polygon(
                        &outline(edge + sidewalk),
                        &[],
                        SIDEWALK_COLOR.to_linear(),
                    );
                }
                kerb_pockets += 1;
            }
        }
    }
    // разворотные площадки в тупиках (`highway=turning_circle`); тупик без
    // тега кончается круглым торцом ленты и так
    let mut turning_circles = 0;
    for node in &map.road_nodes {
        if node.kind != RoadNodeKind::TurningCircle || nodes.is_shared(node.pos) {
            continue;
        }
        let Some(index) = roads.iter().position(|road| {
            road.class == RoadClass::Street
                && !road.carves_navmesh()
                && [road.points.first(), road.points.last()]
                    .into_iter()
                    .flatten()
                    .any(|end| end.distance(node.pos) < JOIN_EPSILON)
        }) else {
            continue;
        };
        let road = prepared.road(index);
        let radius = turning_radius(road.width);
        let disc = |radius: f32| -> Vec<Vec2> {
            (0..TURNING_CIRCLE_SIDES)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / TURNING_CIRCLE_SIDES as f32;
                    node.pos + Vec2::from_angle(angle) * radius
                })
                .collect()
        };
        streets.push_convex(&disc(radius), ROAD_COLOR.to_linear());
        if let Some(sidewalk) = prepared.sidewalk_drawn(index) {
            sidewalks.push_convex(&disc(radius + sidewalk), SIDEWALK_COLOR.to_linear());
        }
        turning_circles += 1;
    }
    // оси ленты, со стежками — до дороги, до которой OSM торец не довёл:
    // асфальт, краска, траектории, острова
    let ribbon = prepared.axes(Axis::Ribbon);
    // траектории манёвров (`roads/turns.rs`) — колея в узле
    let turns = turns::Turns::new(&prepared, &junctions.node_paint().junctions, map.traffic_side);
    // Широкие улицы поверх узких — см. доку модуля; ведущая узла — поверх
    // всех: её колея идёт через узел, и примыкание шире неё не должно её
    // закрыть.
    let mut leading = vec![false; roads.len()];
    for junction in &junctions.node_paint().junctions {
        for &road in &junction.leading {
            leading[road] = true;
        }
    }
    let widths: Vec<f32> = drawn.iter().map(|road| road.width).collect();
    let order = fill_order(&widths, &leading, &junctions.node_paint().junctions);
    // направляющие островки у колец (`roads/gores.rs`) — до лент: к ним
    // дотягиваются двойные сплошные разделительных
    let gore_roads: Vec<gores::GoreRoad> = order
        .iter()
        .filter(|&&index| {
            let road = drawn[index];
            road.class == RoadClass::Street && !road.carves_navmesh()
        })
        .map(|&index| gores::GoreRoad::new(drawn[index], &ribbon[index]))
        .collect();
    let mut gores = gores::Gores::of(&gore_roads);
    // островки по правилу — на двусторонних подходах, где веера из въезда и
    // съезда в OSM нет: краска и колея подхода рвутся на их длину
    let splitters = gores::splitters(&drawn, &ribbon, prepared.rings());
    junctions.add_splitters(&splitters);
    gores.add_splitters(&splitters);
    // три множества разрывов — каждому потребителю своё (`roads/junctions.rs`)
    let node_paint = junctions.node_paint();
    let asphalt = junctions.asphalt();
    let paint_breaks = junctions.paint();
    // каркасы половин у слияний сводятся в каркас продолжения
    // (`roads/merges.rs`) — по той же нарисованной оси, что и линии
    let mut ramps: Vec<Option<paint::MergeRamp>> = vec![None; roads.len()];
    for merge in &prepared.merges().list {
        for (road, ramp) in merges::merge_ramps(merge, &drawn, &ribbon, &map.network, shape.taper())
        {
            ramps[road] = Some(ramp);
        }
    }
    // где у пары половин кончается разделительная — осевая слияния доходит
    // до неё: торцы асфальтовой середины и носы газона, по улицам половин
    let street_of = |road: usize| map.network.street_of(road).map(|(street, _)| street);
    let mut median_ends: Vec<([Option<usize>; 2], merges::MedianEnd)> = Vec::new();
    // разделительные парных половин (`roads/medians.rs`): асфальт — до лент
    // половин, под ними; газон с бордюром — в свой слой над тротуарами
    let mut median_grass = MeshBuilder::with_surface_coords();
    let mut paved: Vec<network::pairs::Median> = Vec::new();
    let mut lawn_kerbs = Vec::new();
    // торцы трамвайных полотен — разрывы для газона рядом: полотно и газон
    // одной пары улиц встречаются торец в торец
    let bed_ends: Vec<Break> = prepared
        .pairs()
        .medians
        .iter()
        .filter(|median| median.carries_tram())
        .flat_map(medians::bed_ends)
        .flatten()
        .collect();
    // разделительная открывается по базе — у перекрёстка, кто бы его ни вёл
    let base = junctions.median_base();
    for median in &prepared.pairs().medians {
        let [first, second] = median.roads;
        let breaks = medians::crossing_breaks(median, [&base[first], &base[second]]);
        // до перекрёстка — как линии полос, а не там, где кончились пробы
        let mut median = median.clone();
        medians::reach_breaks(&mut median, &breaks);
        if median.is_paved() {
            // полотно — внутренние полосы половин до середины; узкая
            // разделительная — полосой асфальта во всё расстояние между осями
            if median.carries_tram() {
                medians::push_bed(&mut streets, &median, ROAD_COLOR.to_linear());
            } else {
                medians::push_paved(&mut streets, &median, ROAD_COLOR.to_linear(), ROAD_JOIN);
            }
            if style.markings {
                let mut midline = median.midline.clone();
                gores.reach(&mut midline);
                // и там, где обе половины рвёт краска узла — зебра поперёк
                // обеих, стоп-линии
                let mut painted = breaks.clone();
                painted.extend(medians::crossing_breaks(
                    &median,
                    [paint_breaks.of(first).cut, paint_breaks.of(second).cut],
                ));
                // узел слияния — не перекрёсток: двойная сплошная доходит до
                // него и переходит в осевую продолжения
                painted.retain(|gap| !prepared.merges().is_pure_node(gap.at));
                painter.paint_median(&midline, &painted);
                let pair = median.roads.map(street_of);
                for tip in [midline.first(), midline.last()].into_iter().flatten() {
                    median_ends.push((pair, merges::MedianEnd::Paved(*tip)));
                }
            }
            paved.push(median);
        } else {
            let mut breaks = breaks;
            breaks.extend(bed_ends.iter().copied());
            let kerbs = medians::push_lawn(
                &mut sidewalks,
                &mut median_grass,
                &median,
                &breaks,
                SIDEWALK_COLOR.to_linear(),
                GRASS_COLOR.to_linear(),
            );
            let pair = median.roads.map(street_of);
            for point in kerbs.iter().flatten().flatten() {
                median_ends.push((pair, merges::MedianEnd::Lawn(Vec2::from(*point))));
            }
            lawn_kerbs.extend(kerbs);
        }
    }
    // за узлом слияния, до разделительной его пары: асфальт до носа газона —
    // под лентами половин — и осевая продолжения
    streets.set_lanes(None);
    for merge in prepared.merges().list.iter().filter(|merge| merge.pure) {
        let halves = merge.halves.map(street_of);
        let ends: Vec<merges::MedianEnd> = median_ends
            .iter()
            .filter(|(pair, _)| {
                pair.iter()
                    .all(|street| street.is_some() && halves.contains(street))
            })
            .map(|&(_, end)| end)
            .collect();
        for shape in merges::nose_fill(merge, &ribbon, &map.network, &ends, &lawn_kerbs) {
            push_shape(&mut streets, shape, ROAD_COLOR.to_linear());
        }
        if style.markings {
            let axis = merges::merge_axis(merge, &drawn, &ribbon, &map.network, &ends);
            painter.paint_merge_axis(&axis, lane_count(drawn[merge.street]));
        }
    }
    // асфальт от торца полотна до носа газона рядом
    if !lawn_kerbs.is_empty() {
        streets.set_lanes(None);
        for bed in paved.iter().filter(|median| median.carries_tram()) {
            for cap in medians::bed_caps(bed, &lawn_kerbs) {
                push_shape(&mut streets, cap, ROAD_COLOR.to_linear());
            }
        }
    }
    let network_time = started.elapsed();

    // щель между подходом и кольцом — асфальтом, под лентами
    for web in &prepared.rings().webs {
        streets.push_polygon(web, &[], ROAD_COLOR.to_linear());
    }
    // остров-крошка в треугольнике узлов — тоже
    for island in &islands {
        streets.push_polygon(island, &[], ROAD_COLOR.to_linear());
    }
    // островки безопасности и площади полотна из данных (`roads/islands.rs`):
    // площадь — асфальтом улиц до лент, под ними (порядок пуша в слое —
    // порядок отрисовки); островок — бордюром поверх асфальта и краски, ниже
    let road_islands = islands::RoadIslands::new(map, &drawn, &ribbon);
    streets.set_lanes(None);
    for shape in &road_islands.carriageways {
        push_shape(&mut streets, shape.clone(), ROAD_COLOR.to_linear());
    }
    if style.sidewalks {
        for ring in &prepared.rings().list {
            let width = drawn[ring.roads[0]].width;
            let sidewalk = ring
                .roads
                .iter()
                .filter_map(|&road| prepared.sidewalk_drawn(road))
                .fold(0.0, f32::max);
            push_ring_edges(
                &mut sidewalks,
                ring,
                [width, sidewalk],
                SIDEWALK_COLOR.to_linear(),
            );
        }
    }
    for index in order {
        let road = drawn[index];
        let color = match road.class {
            RoadClass::Street => ROAD_COLOR,
            RoadClass::Alley => ALLEY_COLOR,
        };
        let points: &[Vec2] = &ribbon[index];
        // колея гаснет по разрывам асфальта; у ведущей узла их там нет
        let breaks = asphalt.of(index);
        let lanes = road_lanes(road);
        // линии краски — по той же оси, разрывам и клиньям, что и асфальт
        if style.markings {
            let wedges = if road.bridge {
                [None; 2]
            } else {
                paint::wedge_ends(points, prepared.tapers(), &drawn, index, map.traffic_side)
            };
            painter.paint(
                road,
                points,
                paint_breaks.of(index),
                wedges,
                node_paint.pockets[index],
                ramps[index],
                stations[index],
            );
        }
        if road.bridge {
            // бордюр и тень — мосту; заливка — здесь, в порядке улиц
            bridges.push_deck(index, points, road);
            let fills = bridges.fills();
            fills.set_lanes(lanes);
            push_street_fill(
                fills,
                points,
                road.width,
                color.to_linear(),
                breaks,
                [false; 2],
            );
            continue;
        }
        let fill = match road.class {
            RoadClass::Street => &mut streets,
            RoadClass::Alley => &mut alleys,
        };
        // клинья у швов со сменой сечения: торцы, срезанные под них, и сами
        // клинья от ширины узкого соседа (`roads/tapers.rs`)
        let ends = prepared.taper_ends(index);
        let [head, body, tail] = if ends == [None; 2] {
            [None, None, None]
        } else {
            tapers::split(points, ends.map(|end| end.map(|taper| taper.length)))
        };
        let body: &[Vec2] = body.as_deref().unwrap_or(points);
        // торец под клин и торец плеча, кончающегося в узле, — прямые: узел
        // закрывают скругления и наружные углы (`roads/corners.rs`); торец
        // со стежком уже не в узле
        let butt = kerb_returns.butt(index);
        let stitched_end = prepared.stitched_end(index);
        let trimmed = [
            head.is_some() || (butt[0] && !stitched_end[0]),
            tail.is_some() || (butt[1] && !stitched_end[1]),
        ];
        let wedges: Vec<(&[Vec2], tapers::Taper, bool)> =
            [(&head, ends[0], false), (&tail, ends[1], true)]
                .into_iter()
                .filter_map(|(path, taper, end)| Some((path.as_deref()?, taper?, end)))
                .collect();
        // «до разрыва» клина — продолжение срезанной ленты за её торцом: от
        // узла шва до стыка с лентой, чтобы штрихи шли через стык без сдвига
        let continued = |path: &[Vec2], width: f32, breaks: &[Break], end: bool| {
            let length = polyline_length(path);
            [
                to_break_beyond(body, width, breaks, end, length),
                to_break_beyond(body, width, breaks, end, 0.0),
            ]
        };

        // тротуар кольца — одной лентой на всё кольцо (`push_ring_edges`)
        let ring = prepared.rings().of(index);
        if let Some(sidewalk) = prepared.sidewalk_drawn(index).filter(|_| ring.is_none()) {
            let band = |road: &RoadLine, sidewalk: f32| road.width + 2.0 * sidewalk;
            // у половины разделённой улицы тротуара со стороны пары нет; на
            // клине куски пары не пересчитываются — там тротуар как был
            let runs = if wedges.is_empty() {
                prepared.pairs().runs[index].as_slice()
            } else {
                &[]
            };
            let stitch = prepared.stitch_offset(index);
            push_sidewalk(
                &mut sidewalks,
                body,
                [road.width, sidewalk],
                runs,
                stitch,
                road.sidewalk().sides(),
                SIDEWALK_COLOR.to_linear(),
                trimmed,
            );
            // клин тротуара — по сторонам, как у асфальта: с сужаемой стороны
            // от полосы узкого соседа (или его голой кромки, если тротуара у
            // него нет) к своей, с сохранённой — своя на всём клине; сторона
            // без тротуара по тегу — голая кромка (`Drawn::band_half`)
            for &(path, taper, end) in &wedges {
                let halves = wedge_halves(taper.sides, end, |side| {
                    [
                        prepared.band_half(taper.narrow, side),
                        prepared.band_half(index, side),
                    ]
                });
                sidewalks.push_taper_sided(
                    path,
                    halves,
                    continued(path, band(road, sidewalk), &[], end),
                    SIDEWALK_COLOR.to_linear(),
                );
            }
        }
        // Клин половины разделённой улицы сужается и со стороны пары, а
        // разделительная считана по полной ширине: в щели между ними лежал
        // полный тротуар половины — светлая полоса с тёмной кромкой вдоль
        // всего клина (пример 16). Со стороны пары под клин кладётся асфальт
        // полной полуширины — кромка там идёт прямо, как у тела.
        let length = polyline_length(points);
        for &(path, taper, end) in &wedges {
            let middle = if end {
                length - polyline_length(path) / 2.0
            } else {
                polyline_length(path) / 2.0
            };
            let Some(run) = prepared.pairs().runs[index]
                .iter()
                .find(|run| (run.from..=run.to).contains(&middle))
            else {
                continue;
            };
            // с сохранённой стороны кромка и так прямая
            if !taper.sides[usize::from(!run.left)] {
                continue;
            }
            let side = if run.left { 1.0 } else { -1.0 };
            let inner: Vec<Vec2> = path
                .iter()
                .zip(miter_offsets(path, false, road.width / 4.0))
                .map(|(&point, offset)| point + offset * side)
                .collect();
            fill.set_lanes(None);
            push_ribbon_trimmed(
                fill,
                &inner,
                road.width / 2.0,
                color.to_linear(),
                ROAD_JOIN,
                [true; 2],
            );
        }
        // у узла слияния колея плывёт за линиями краски — по той же рампе;
        // профиль — по длине `points`, а тело начинается за клином у начала
        match (lanes, ramps[index]) {
            (Some(frame), Some(ramp)) => {
                let head = head.as_deref().map_or(0.0, polyline_length);
                let profile = ramp
                    .lane_profile(frame, polyline_length(points))
                    .into_iter()
                    .map(|(along, frame)| (along - head, frame))
                    .collect();
                fill.set_lane_profile(lanes, profile);
            }
            _ => fill.set_lanes(lanes),
        }
        push_street_fill(fill, body, road.width, color.to_linear(), breaks, trimmed);
        fill.set_lanes(lanes);
        for &(path, taper, end) in &wedges {
            let narrow = drawn[taper.narrow];
            let to_break = continued(path, road.width, breaks, end);
            // раскладка плывёт от сечения соседа к своему — та же, что у
            // линий краски на этом клине
            match lanes {
                Some(_) => {
                    let wedge = paint::WedgeEnd {
                        length: polyline_length(path),
                        lanes: lane_count(narrow),
                        drift: paint::wedge_drift(road, map.traffic_side),
                        kept: taper.kept(),
                    };
                    let [from, to] = paint::wedge_frames(lane_count(road), wedge, end);
                    fill.set_lane_taper(Some(from), Some(to));
                }
                None => fill.set_lanes(None),
            }
            fill.push_taper_sided(
                path,
                wedge_halves(taper.sides, end, |_| [narrow.width / 2.0, road.width / 2.0]),
                to_break,
                color.to_linear(),
            );
        }
        if road.class == RoadClass::Street && !road.passage {
            grounds.push(road, points);
        }
    }
    for zebra in &node_paint.zebras {
        painter.paint_zebra(zebra);
    }
    for line in &node_paint.stop_lines {
        painter.paint_stop_line(line);
    }
    // колея траекторий — всегда, как колея полос
    painter.paint_turn_wear(&turns.wear);
    // стрелки на полосах подходов — краска, своим тумблером
    if style.arrows {
        let marks = paint::ArrowMarks::new(&node_paint.zebras, &node_paint.stop_lines);
        for arrow in &turns.arrows {
            let setback = paint::Painter::arrow_setback(arrow, &marks);
            painter.paint_arrow(arrow, setback);
            // и второй ряд дальше от узла, где полоса это позволяет
            let breaks = paint_breaks.of(arrow.road).cut;
            if let Some(repeat) = paint::Painter::repeat_setback(arrow, setback, &marks, breaks) {
                painter.paint_arrow(arrow, repeat);
            }
        }
    }
    // направляющие островки у колец: асфальт — в слой улиц, поверх тротуаров,
    // разметка — в слой краски, своим мешем выше асфальта стоянок
    // (`roads/gores.rs`)
    gores.push_asphalt(&mut streets, ROAD_COLOR.to_linear());
    // светлая полоса над рельсами (`roads/tram_band.rs`) — поверх всего
    // асфальта улиц: порядок пуша в слое — порядок отрисовки, а краска лежит
    // своим слоем выше
    let tram_bands = tram_band::tram_bands(&map.rails, &drawn, &nodal, &paved);
    streets.set_lanes(None);
    // одной фигурой, со щелями между полосами соседних путей заросшими
    for shape in tram_band::band_cover(&tram_bands) {
        push_shape(&mut streets, shape, TRAM_BAND_COLOR.to_linear());
    }
    let mut lot_layers = grounds.layers(&style, &gores, &paved);
    for shape in &road_islands.kerbs {
        push_shape(
            &mut lot_layers.sidewalks,
            shape.clone(),
            SIDEWALK_COLOR.to_linear(),
        );
    }
    if style.markings {
        for (island, across) in gores.islands() {
            painter.paint_island(island, across);
        }
    }

    let bridge_count = bridges.count();

    let fortresses = Fortresses::of(&map.buildings);
    for wall in walls {
        for run in fortresses.bare_runs(&wall.points) {
            push_ribbon(
                &mut wall_ribbons,
                &run,
                wall.width,
                WALL_COLOR.to_linear(),
                ROAD_JOIN,
            );
        }
    }

    // асфальт, тротуар и дорожка — фактурные, лента стены — плоская; три
    // мостовых слоя со своими высотами и материалами отдаёт `Bridges`
    let mut layers: Vec<LayerMesh> = [
        (
            alleys,
            Z_ALLEY,
            "alleys",
            MaterialSpec::Surface(SurfaceKind::Alley),
        ),
        (
            sidewalks,
            Z_SIDEWALK,
            "sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            median_grass,
            Z_ROAD_MEDIAN,
            "road_medians",
            MaterialSpec::Surface(SurfaceKind::Grass),
        ),
        (
            streets,
            Z_ROAD,
            "roads",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (
            lot_layers.sidewalks,
            Z_LOT_SIDEWALK,
            "lot_sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            lot_layers.lines,
            Z_LOT_LINES,
            "lot_lines",
            MaterialSpec::Flat,
        ),
        (wall_ribbons, Z_WALL, "walls", MaterialSpec::Flat),
    ]
    .into_iter()
    .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
    .chain(bridges.layers())
    .collect();
    let paint_lines = painter.lines;
    let paint_layers = painter.layers();
    let paint_vertices = paint_layers
        .iter()
        .map(|layer| layer.builder.vertex_count())
        .sum();
    // краска — сразу над своим асфальтом: улиц над улицами, мостов над
    // настилом; сортировка устойчива, прочие слои порядка не меняют
    layers.extend(paint_layers);
    layers.sort_by(|a, b| a.z.total_cmp(&b.z));

    let report = RoadReport {
        style,
        junctions: junctions.count(),
        paint_lines,
        paint_vertices,
        zebras: [
            node_paint.zebras.len(),
            node_paint.zebras.iter().filter(|zebra| zebra.osm).count(),
        ],
        stop_lines: node_paint.stop_lines.len(),
        pockets: node_paint.pockets.iter().flatten().flatten().count(),
        clusters: node_paint.clusters,
        kerb_pockets,
        turning_circles,
        through: node_paint.through,
        turns: turns.maneuvers,
        arrows: if style.arrows { turns.arrows.len() } else { 0 },
        leading: leading.iter().filter(|&&lead| lead).count(),
        kerb_returns: kerb_returns.roads.len() - kerb_returns.outer[0],
        sidewalk_returns: kerb_returns.sidewalks.len() - kerb_returns.outer[1],
        outer_corners: kerb_returns.outer,
        drawn: prepared.stats(),
        bridges: bridge_count,
        gores: gores.count(),
        road_islands: [
            road_islands.refuges,
            road_islands.kerbs.len() - road_islands.refuges,
            road_islands.carriageways.len(),
        ],
        merge_edges,
        islands: islands.len(),
        tram_bands: tram_bands.len(),
        vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
        network: network_time,
        elapsed: started.elapsed(),
    };
    (layers, report)
}

/// Офлайн-замер дорожных слоёв — строками `LayerCost`, как у зданий и машин.
///
/// Своей сборки у него нет: он зовёт тот же [`mesh_roads`], что и игра. До шва
/// дорожные слои мерились только строкой `road meshing:` из живого приложения,
/// то есть ровно тем способом, который на macOS врёт (App Nap).
pub fn measure_roads(map: &MapData) -> Vec<LayerCost> {
    let (layers, report) = mesh_roads(map, RoadStyle::default(), RoadShape::default());
    // счётчики сети (стыки, скругления, островки) в строках слоёв не видны —
    // та же строка, что пишет в лог игра
    eprintln!("{report}");
    surface::layer_costs(&layers, report.elapsed)
}

/// Когда пересобирать дорожные слои — **условие живёт рядом со слоем**, а не у
/// того, кто ставит систему в расписание: причина тут дорожная, и узнать её
/// надо, правя `roads.rs`, а не `map/mod.rs`.
///
/// `SunOnMap` в списке потому, что **в дорожный меш запечена тень моста**:
/// настил, сдвинутый по `shadow_dir()` на высоту пролёта через
/// `shadow_length_scale()`. Без этого условия она осталась бы от солнца, с
/// которым грузился город, пока все остальные тени карты едут за осевшим.
/// Осевшим (`SunOnMap`), а не ползунком (`SunStyle`): на шкале семьдесят
/// делений, и каждое стоило бы полной пересборки девяти слоёв.
///
/// **Условие одно, регистрация одна.** Две копии одной системы в одном
/// расписании могут сработать в одном кадре обе, и слой заспавнится дважды:
/// деспавн второй копии идёт по данным, снятым до применения команд первой.
/// Поэтому условия складываются через `or_else`, а не разносятся по
/// регистрациям.
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<RoadStyle>
        .or_else(retuned::<RoadShapeOnMap>)
        .or_else(retuned::<SunOnMap>)
}

/// Пересборка дорожных слоёв после переключения стиля из UI или BRP: деспавн
/// старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    style: Res<RoadStyle>,
    shape: Res<RoadShapeOnMap>,
    map: Res<MapData>,
    existing: Query<Entity, With<RoadLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        mesh_roads(&map, *style, shape.0),
    );
}

/// Положить в мир то, что собрал [`mesh_roads`]: слои под `RoadLayerTag`, плюс
/// отчёт в лог. Одна дверь для `rebuild_roads` и `spawn_map` — форма
/// `buildings::spawn_building_meshes`: дверь нужна не по числу меток, а по
/// числу вызывающих.
pub fn spawn_road_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    (layers, report): (Vec<LayerMesh>, RoadReport),
) {
    // меши краски несут ещё и свой вид — по нему их прячет ступень зума
    // (`paint::show_paint`)
    let (paint, rest): (Vec<LayerMesh>, Vec<LayerMesh>) = layers
        .into_iter()
        .partition(|layer| paint::PaintTag::of(layer.name).is_some());
    spawn_layers(commands, meshes, materials, rest, RoadLayerTag);
    for layer in paint {
        let Some(tag) = paint::PaintTag::of(layer.name) else {
            continue;
        };
        spawn_layers(commands, meshes, materials, [layer], (RoadLayerTag, tag));
    }
    info!("{report}");
}

/// Лента выбранного стиля — `RoadJoin` через [`RoadJoin::ribbon_shape`]. Общая
/// с подложкой аллей (`map::spawn`): у неё те же три настройки, что у дорог, и
/// мапиться на `MeshBuilder` они обязаны одинаково.
///
/// Слои с **жёстко заданным** стыком (ограда, рельсы, трамвай) зовут
/// `MeshBuilder::push_ribbon` напрямую: обёртка им говорила бы только «переведи
/// `RoadJoin::Round` в `RibbonJoin::Round`», то есть выдавала бы стиль дорог за
/// их собственный.
///
/// **Замкнутый путь рисуется замкнутой лентой** — признак берётся из самого
/// пути ([`is_ring`]), а не аргументом: у кольца торцов нет, и решать это за
/// ленту нечем, кроме её же формы. Иначе на шве ложились два торцевых
/// полудиска поверх собственного асфальта кольца, и внутри такого диска
/// разметка с износом считались в **замороженной** раме торца — круглое пятно
/// со смещёнными штрихами (отчёт автора по кольцу ТРЦ «Макси»).
pub fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    push_ribbon_trimmed(builder, points, width, color, join, [false; 2]);
}

/// Лента дороги, у которой торец `[начало, конец]` может быть **срезан под
/// клин** (`roads/tapers.rs`): такой торец кончается ровно на своей точке.
/// Круглый торец полной ширины выпер бы из-под клина, который в этом месте
/// начинается с той же ширины и дальше только сужается.
fn push_ribbon_trimmed(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
    trimmed: [bool; 2],
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    let caps = trimmed.map(|trimmed| if trimmed { RibbonCap::Butt } else { cap });
    builder.push_ribbon_capped(points, is_ring(points), width, color, join, caps);
}

/// Полуширины клина для `MeshBuilder::push_taper_sided` — `[[слева в начале,
/// слева в конце], [справа …]]` в раме **пути клина** (от шва к телу):
/// `half(сторона по ходу way)` — полуширина `[узкого соседа, своя]` с той
/// стороны; сужаемая сторона (`sides`, `roads/tapers.rs`) идёт от первой ко
/// второй, сохранённая — своя на всём клине. У торца конца путь идёт против
/// way, и стороны меняются местами.
fn wedge_halves(sides: [bool; 2], end: bool, half: impl Fn(usize) -> [f32; 2]) -> [[f32; 2]; 2] {
    let by_side = [0, 1].map(|side| {
        let [narrow, own] = half(side);
        if sides[side] { [narrow, own] } else { [own; 2] }
    });
    if end {
        [by_side[1], by_side[0]]
    } else {
        by_side
    }
}

/// Тротуар дороги шириной `widths[0]` с полосой `widths[1]` — с тех сторон
/// `[слева, справа]`, где он есть по тегу (`sides`, [`RoadLine::sidewalks`]),
/// и кроме кусков `runs`, где рядом идёт вторая половина разделённой улицы
/// (`roads/network/pairs.rs`): со стороны пары его нет. Длины кусков меряны
/// по оси без стежка; `stitch` — длина стежка перед её началом.
#[allow(clippy::too_many_arguments)]
fn push_sidewalk(
    builder: &mut MeshBuilder,
    body: &[Vec2],
    [width, sidewalk]: [f32; 2],
    runs: &[network::pairs::PairRun],
    stitch: f32,
    sides: [bool; 2],
    color: LinearRgba,
    trimmed: [bool; 2],
) {
    if runs.is_empty() && sides == [true; 2] {
        return push_ribbon_trimmed(
            builder,
            body,
            width + 2.0 * sidewalk,
            color,
            ROAD_JOIN,
            trimmed,
        );
    }
    let total = polyline_length(body);
    let mut cursor = 0.0;
    // кусок `from..to` с тротуаром по сторонам `[слева, справа]`
    let mut piece = |from: f32, to: f32, [left, right]: [bool; 2]| {
        if to - from < SIDEWALK_PIECE_MIN {
            return;
        }
        let points = tapers::cut(body, from, to);
        let trims = [from > 0.0 || trimmed[0], to < total || trimmed[1]];
        // полоса с одной стороны — лента на полтротуара в её сторону:
        // `miter_offsets` плюсом сдвигает влево
        let shift = match (left, right) {
            (true, true) => {
                return push_ribbon_trimmed(
                    builder,
                    &points,
                    width + 2.0 * sidewalk,
                    color,
                    ROAD_JOIN,
                    trims,
                );
            }
            (true, false) => sidewalk / 2.0,
            (false, true) => -sidewalk / 2.0,
            (false, false) => return,
        };
        let shifted: Vec<Vec2> = points
            .iter()
            .zip(miter_offsets(&points, false, shift))
            .map(|(point, offset)| *point + offset)
            .collect();
        push_ribbon_trimmed(builder, &shifted, width + sidewalk, color, ROAD_JOIN, trims);
    };
    let mut previous: Option<bool> = None;
    for run in runs {
        let from = (run.from + stitch).clamp(cursor, total);
        let to = (run.to + stitch).clamp(from, total);
        // со стороны пары тротуара нет
        let mut paired = sides;
        paired[usize::from(!run.left)] = false;
        // и в щели между двумя кусками с той же стороны — любыми, полотном и
        // газоном тоже, — которые разделительные сводят торец в торец
        // (`Pairs::join_ends`): светлое пятно тротуара лежало между ними
        let bridged = previous == Some(run.left) && from - cursor < network::pairs::JOIN_GAP;
        piece(cursor, from, if bridged { paired } else { sides });
        piece(from, to, paired);
        cursor = to;
        previous = Some(run.left);
    }
    piece(cursor, total, sides);
}

/// Заливка проезжей части — лента [`ROAD_RIBBON`] с разрывами разметки по
/// перекрёсткам. Срезанный под клин торец — как у [`push_ribbon_trimmed`].
fn push_street_fill(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    breaks: &[Break],
    trimmed: [bool; 2],
) {
    let (join, cap) = ROAD_RIBBON;
    builder.push_ribbon_shaped(
        points,
        width,
        color,
        RibbonShape {
            closed: is_ring(points),
            join,
            caps: trimmed.map(|trimmed| if trimmed { RibbonCap::Butt } else { cap }),
            breaks: RibbonBreaks::At(breaks),
        },
    );
}

/// Осевая дороги вне улицы: моста, арки, дорожки (улицы идут по
/// [`axis::street_axes`]). Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
///
/// **Общие узлы с другими дорогами тоже не сглаживаются** ([`RoadNodes`]): на
/// узле кончается поперечная улица и сходятся лучи скругления бордюра
/// (`roads/corners.rs`). Сдвинь хорда сквозную дорогу с узла — торец
/// поперечной повис бы в метре от её асфальта или вылез за дальний край.
///
/// **Замкнутый way сглаживается по циклу**, так что нарисованная ось кольца
/// остаётся кольцом: её потом и рисуют замкнутой лентой ([`push_ribbon`],
/// [`push_street_fill`]).
fn centerline<'a>(road: &'a RoadLine, smoothing: Smoothing, nodes: &RoadNodes) -> Cow<'a, [Vec2]> {
    if road.passage {
        return Cow::Borrowed(&road.points);
    }
    smooth_pinned(
        &road.points,
        road.width,
        smoothing,
        is_ring(&road.points),
        |point| nodes.is_shared(point),
    )
}

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин обязан
/// рваться на тех же перекрёстках, на которых рвётся разметка, и второго
/// восстановления узлов по общим нодам заводить незачем.
pub(super) mod junctions;

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин стоит на той
/// же оси, что и лента.
pub(super) mod axis;
mod bridges;
mod corners;
mod drawn;
mod gores;
mod islands;
mod lots;
mod medians;
mod merges;
/// Открыт наружу для [`map::footprint`](crate::map::footprint): проём в ограде
/// у брошенного торца — тот же вопрос «висячий ли он», что у стежка, и второго
/// ответа на него быть не должно. И для разбора: улицы и сечения
/// (`network::sections`) собираются там, потому что ширину дороги читают
/// следующие проходы разбора, — а сеть потом лежит в `MapData::network`.
pub mod network;
pub mod node_paint;
pub mod paint;
/// Открыт наружу для [`map::cars`](crate::map::cars): машина встаёт в тот же
/// карман, что кладёт лента.
pub(super) mod pockets;
mod rings;
/// Открыт наружу для панели и витрины: ресурс ручек формы и глобаль ширины
/// полосы.
pub mod shape;
/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин прерывается
/// на клине, где бордюр ближе к оси, чем полуширина участка.
pub(super) mod tapers;
mod tram_band;
mod turns;

#[cfg(test)]
mod tests;
