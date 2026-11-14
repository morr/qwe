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
//! [`drawn::centerline`], дорожная обёртка над ним с её двумя закреплениями.
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

use bevy::prelude::*;

pub use self::bridges::BridgeReport;
use self::bridges::Bridges;
pub use self::drawn::{Axis, Drawn, DrawnStats};
use self::gores::splitters::Splitter;
use self::islands::push_street_fill;
pub use self::junctions::JunctionCounts;
#[cfg(test)]
use self::junctions::turning_radius;
use self::junctions::{Junctions, MOUTH_CROSSING_GAP, mouths};
pub use self::node_paint::CrossingMode;
use self::paint::Station;
pub use self::ribbons::push_ribbon;
use self::ribbons::{wedge_halves, wedge_middle};
use self::rings::{ApproachBand, ring_island_grass, ring_island_lawns};
use self::ruts::RutLines;
pub use self::shape::{ROAD_JOIN, RoadJoin, RoadStyle};
use self::shape::{ROAD_RIBBON, RoadShape, RoadShapeOnMap};
use self::tram_lay::TramTracks;
use self::turns::Turns;
pub(crate) use self::verges::paved_verge;
pub use self::verges::{VergeLods, VergeZoomBucket};
#[cfg(test)]
use self::walls::Fortresses;
use self::walls::Meadows;
pub(crate) use self::walls::Z_WALL;
use crate::map::SunOnMap;
use crate::map::meshing::{LaneFrame, MeshBuilder, RibbonCap, RibbonJoin};
#[cfg(test)]
use crate::map::osm::model::RoadNodeKind;
use crate::map::osm::model::{RoadAreaKind, polyline_length};
use crate::map::osm::{MapData, RailKind, RoadClass, RoadLine};
use crate::map::parallel::{in_parallel, in_parallel_owned};
use crate::map::rail::{CrossedStreet, crossable, mesh_level_crossings};
use crate::map::shapes::{is_ring, push_shape};
use crate::map::surface::{
    self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, SurfaceKind, spawn_layers,
};
use crate::map::zoom::{LodStep, ZoomLods};
use crate::prefs::retuned;
use crate::settings::{
    Z_ALLEY, Z_LOT_LINES, Z_LOT_SIDEWALK, Z_RAIL_CROSSING, Z_RING_GRASS, Z_RING_ISLAND, Z_ROAD,
    Z_ROAD_MEDIAN, Z_ROAD_VERGE, Z_ROAD_VERGE_LAWN, Z_ROAD_VERGE_YARD, Z_SIDEWALK, Z_UNPAVED_ROAD,
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
/// Грунтовая улица — серо-бурый утрамбованный щебень: светлее асфальта на
/// ступень и теплее его, темнее песчаной тропинки, чтобы проезжая часть
/// частного сектора читалась дорогой, а не дорожкой.
const UNPAVED_ROAD_COLOR: Color = Color::srgb(0.64, 0.6, 0.53);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Белая разметка на асфальте стоянки: двойная сплошная между встречными
/// полотнами (`roads/lots.rs`), слой `lot_lines`. Обводка со штриховкой
/// направляющего островка (`roads/gores.rs`) — не здесь: она в слое краски
/// `road_paint_islands`.
const LOT_LINE_COLOR: Color = Color::srgb(0.88, 0.88, 0.86);

/// Тротуар — светлый бетон между асфальтом и тёплой землёй: светлее проезжей
/// части на четверть, и именно эта ступень яркости читается как бордюр.
const SIDEWALK_COLOR: Color = Color::srgb(0.82, 0.815, 0.80);

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
fn fill_order(
    widths: &[f32],
    leading: &[bool],
    junctions: node_paint::PaintJunctions<'_>,
) -> Vec<usize> {
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
    for junction in junctions.iter() {
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
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RoadReport {
    /// Стиль, которым всё это нарисовано: тумблеры из лог-строки — это он.
    pub style: RoadStyle,
    /// Узлы (`roads/junctions.rs`): перекрёстки, кластеры, проходы главной
    /// насквозь, ведущие дороги, зебры (из них по OSM), стоп-линии, карманы
    /// краски — одним значением.
    pub junctions: JunctionCounts,
    /// Куски линий слоя краски (`roads/paint.rs`) и их вершины — отдельно от
    /// общего счёта: краска строится своими мешами и прячется с зумом.
    pub paint_lines: usize,
    pub paint_vertices: usize,
    /// Карманы стоянки вдоль улиц (`roads/pockets.rs`) и разворотные
    /// площадки в тупиках.
    pub kerb_pockets: usize,
    pub turning_circles: usize,
    /// Траектории узлов (`roads/turns.rs`) — кривые манёвров.
    pub turns: usize,
    /// Стрелки на полосах подходов (`roads/turns.rs`).
    pub arrows: usize,
    pub kerb_returns: usize,
    pub sidewalk_returns: usize,
    /// Наезды плитки на дворе у слияний мощёных дорожек
    /// (`corners::tile_overlaps`).
    pub tile_overlaps: usize,
    /// Наружные углы узлов (`roads/corners.rs`): асфальт и тротуар.
    pub outer_corners: [usize; 2],
    /// Носы острых развилок (`roads/corners.rs`), всех слоёв.
    pub noses: usize,
    /// Изогнутые скругления (`corners::bend`) — у колец и гнутых подходов.
    pub bends: usize,
    /// Лоскуты газона обочин, замощённые плиткой (`roads/scraps.rs`).
    pub lawn_scraps: usize,
    /// Подготовка дорог (`roads/drawn.rs`): переезды, стежки, клинья,
    /// слияния, разделительные, кольца, швы осей — одним значением.
    pub drawn: DrawnStats,
    /// Мосты (`roads/bridges.rs`): мостовых ways, мостов-цепочек из них и
    /// мостов с тенью.
    pub bridges: BridgeReport,
    /// Путепроводы (`Bridges::push_track`) и настилы переездов в одном уровне
    /// (`rail::mesh_level_crossings`, пар звеньев путь × улица).
    pub track_bridges: usize,
    pub level_crossings: usize,
    /// Острова-крошки в треугольниках узлов, залитые асфальтом
    /// (`corners::small_islands`).
    pub islands: usize,
    /// Направляющие островки у колец (`roads/gores.rs`).
    pub gores: usize,
    /// Из данных v15 (`roads/islands.rs`): островков-точек на улицах, контуров
    /// островков, контуров полотна и пешеходных площадей.
    pub road_islands: [usize; 4],
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
            junctions:
                JunctionCounts {
                    count: junctions,
                    clusters,
                    through,
                    leading,
                    zebras: [zebras, osm_zebras],
                    stop_lines,
                    pockets,
                },
            paint_lines,
            paint_vertices,
            kerb_pockets,
            turning_circles,
            turns,
            arrows,
            kerb_returns,
            sidewalk_returns,
            tile_overlaps,
            outer_corners: [outer, outer_sidewalks],
            noses,
            bends,
            lawn_scraps,
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
            track_bridges,
            level_crossings,
            islands,
            gores,
            road_islands: [refuges, island_areas, carriageways, walkways],
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
             {sidewalk_returns} on sidewalks, tile overlaps {tile_overlaps}, outer corners {outer} + {outer_sidewalks} on \
             sidewalks, noses {noses}, bends {bends}, lawn scraps {lawn_scraps}, stitches {stitches}, kerb pockets {kerb_pockets}, turning circles {turning_circles}, driveway crossings \
             {crossings}, rings {rings} ({webs} webs), small islands {islands}, gores {gores}, safety islands {refuges} + {island_areas} areas, \
             carriageway areas {carriageways}, walkway areas {walkways}, tapers {tapers}, merges {merges} ({merge_edges} edges), medians {paved} paved + {lawns} \
             lawn (tram beds {beds}), tram bands {tram_bands}, smooth seams {seams}, tight corners {tight}, bridges {bridges} of \
             {bridge_ways} ways ({casting} cast shadows), track bridges {track_bridges}, level \
             crossings {level_crossings}; {network:?} of it before the \
             ribbons)",
            style.sidewalks, style.markings,
        )
    }
}

/// Двадцать пять дорожных слоёв в выбранном стиле и форме: газон островов
/// колец, два газона широких обочин с плиткой у бордюра, замапленная трава
/// острова, заливка аллей, тротуары, газон разделительных, грунтовые улицы,
/// заливка улиц, два слоя большой стоянки, лента крепостной стены, настилы
/// переездов, три мостовых слоя и восемь слоёв краски
/// (`roads/paint.rs`) над своим асфальтом.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_roads`] и `spawn_map`), и тест. Это самый
/// крупный модуль шва, и он же самый показательный: двадцать пять слоёв,
/// четыре вида материала и вся телеметрия области — всё уезжает через один
/// возврат.
/// `verge` — ступень зума обочин ([`VergeLods`]): по правилу шва ступень, а
/// не развёрнутое из таблицы значение.
pub fn mesh_roads(
    map: &MapData,
    style: RoadStyle,
    shape: RoadShape,
    verge: VergeZoomBucket,
) -> (Vec<LayerMesh>, RoadReport) {
    let RoadBuild { layers, report, .. } = build_roads(map, style, shape, verge);
    (layers, report)
}

/// Нарисованное мощение углов контурами — для покрытия карманов парсa
/// (`osm/parse/pockets.rs`): скругления улиц, тротуаров и обочин, мощёные
/// носы и дуги ([`corners::KerbReturns::paving`]). Полосы покрытия карманов
/// меряются от осевых и скруглений не знают, поэтому в горлах перекрёстков
/// щель покрытия ложится поверх нарисованного мощения, и карман засевается
/// там, где на карте асфальт (Свободы × Первомайская). Стиль и форма —
/// дефолтные: карманы считаются раньше, чем мир узнаёт ручки стиля, а
/// покрытие и так аппроксимация рисунка. Возвращает копии колец.
pub fn corner_paving_contours(map: &MapData) -> Vec<Vec<Vec2>> {
    let style = RoadStyle::default();
    let shape = RoadShape::default();
    let drawn = Drawn::new(map, &style, &shape);
    let returns = corners::kerb_returns(
        &drawn,
        shape.corner_radius(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let paving = returns.paving();
    paving
        .flat()
        .chain(paving.curved())
        .map(<[Vec2]>::to_vec)
        .collect()
}

/// Всё, что выходит из одной сборки дорожной области ([`build_roads`]):
/// слои и отчёт — то же, что отдаёт [`mesh_roads`], — и три побочных
/// результата того же прохода, которые игра кладёт ресурсами.
#[derive(Default)]
pub struct RoadBuild {
    pub layers: Vec<LayerMesh>,
    pub report: RoadReport,
    /// Линии колеи: по ним строится оверлей колеи (`roads/ruts.rs`).
    pub ruts: RutLines,
    /// Трамвайные пути по нарисованной улице: их читает слой трамвая
    /// (`map/tram.rs`).
    pub tram: TramTracks,
    /// Отрисованные куски: их читает семантический индекс
    /// (`map/inspect.rs`), чтобы находка отвечала кускам ступени этой
    /// сборки, а не текущему зуму ([`DrawnPieces`] — об оговорке).
    pub pieces: Vec<DrawnPiece>,
}

/// Вид отрисованного куска дороги: из него берутся подпись и слой находки
/// ([`DrawnPiece::z`] пишется рядом при сборке, из константы того же слоя).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieceKind {
    /// Плитка обочины: бордюрная полоса, площадки у бордюра и скругления
    /// углов, замостенные лоскуты — всё, что лежит в слое `road_verges`.
    VergePaving,
    /// Луг широкой обочины у замапленного газона (слой `road_verge_lawns`).
    VergeLawn,
    /// Трава двора на широкой обочине (слой `road_verge_yards`).
    VergeYard,
    /// Замостенный клин в зелени земли: карман разбора или замапленный
    /// газон, чей клин конвейер лоскутов замостил (слой тротуаров).
    PavingScrap,
    /// Асфальтовый кусок узла: внешние углы, заливки скруглений, носы и
    /// дуги — то, что лежит за полосами полотна (`corners`).
    RoadCorner,
    /// Тротуарный кусок узла: поворот полосы за бордюром и наезды плитки.
    SidewalkCorner,
}

/// Отрисованный кусок дороги: контур с дырками, вид, дорога-владелец и слой
/// (z), в котором кусок лежит. Собирается тем же проходом [`build_roads`],
/// что и рисует слои, — каждым логическим куском (полоса обочины на ран,
/// лоскут, кусок узла), а не посегментными квадрами меша. Читается
/// семантическим индексом (`map/inspect.rs`): находка отвечает кускам
/// ступени последней пересборки, не текущему зуму ([`DrawnPieces`] —
/// об оговорке), и подсветка ведёт контур куска.
#[derive(Clone)]
pub struct DrawnPiece {
    pub contour: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
    pub kind: PieceKind,
    /// Индекс дороги-владельца в `MapData::roads`: его OSM id ведёт хвост
    /// находки. У узловых кусков (`corners`) владельца нет — угол принадлежит
    /// перекрёстку, а не одной дороге.
    pub road: Option<u32>,
    /// Слой куска: z находки, та же константа, что у слоя при спавне.
    pub z: f32,
}

/// Побочный выход сборки дорожной области: все отрисованные куски. Ресурс
/// записывают вход в мир и пересборка по [`rebuilds_on`] — и куски отвечают
/// ступени обочин той сборки, а не текущему зуму: пересечение порога
/// переключает лишь видимость заранее собранных слоёв ([`show_road_steps`])
/// и ресурс не меняет. После пересборки на ближней ступени находки
/// продолжают называть широкие ленты обочин, скрытые на дальней; после
/// пересборки на дальней (стиль меняют и на дальнем зуме) — не знают лент,
/// которые зум-ин показывает. Ресурс рядом с [`RutLines`] и [`TramTracks`];
/// по его смене перестраивается семантический индекс.
#[derive(Resource, Default)]
pub struct DrawnPieces(pub Vec<DrawnPiece>);

/// [`mesh_roads`] и заодно **линии колеи** ([`RutLines`]) — оси полос с
/// раскладкой, по которой легла колея асфальта, и траектории узлов, по которым
/// её положил `Painter::paint_turn_wear`. Тем же проходом, а не пересчётом:
/// оверлей колеи (Debug → Overlays → `Rut lines`) показывает ровно то, что
/// нарисовано. Игра берёт эту дверь (`rebuild_roads`, `spawn_map`) и кладёт
/// линии ресурсом; тестам и витрине хватает [`mesh_roads`].
///
/// Тем же проходом — и **трамвайные пути по нарисованной улице**
/// ([`TramTracks`], `roads/tram_lay.rs`): по ним лежит светлая полоса над
/// рельсами здесь же, а ресурсом их читает слой трамвая (`map/tram.rs`).
///
/// Тело разбито на **именованные фазы** — баннеры `// ── имя ──` в порядке
/// прохода; самописные куски с узким входом вынесены во вложенные функции
/// (блок «тела вынесенных фаз» перед сборкой), проводка общего состояния —
/// ленты, разделительные, сборка слоёв — остаётся секциями под баннером.
/// Порядок фаз перечислен в скиле `osm-map`, `references/roads.md`.
///
/// Сборка стоит из **скелета** ([`road_skeleton`]) — сети, узлов, краски
/// узлов, всего, что у ступеней общее, — и **финала** ([`build_roads_from`])
/// — обочинных кусков и укладки, что у каждой ступени своё. Витрины зовут её
/// через эту дверь; игра и поток загрузки ходят сплит-дверью
/// ([`build_roads_steps`] в `map/prepare.rs`) — общие слои в мире один раз.
pub fn build_roads(
    map: &MapData,
    style: RoadStyle,
    shape: RoadShape,
    verge: VergeZoomBucket,
) -> RoadBuild {
    // сырой OSM, второй уровень: ни одной достройки отрисовки — и колеи нет,
    // и пути трамвая лежат, где их провёл картограф
    if map.knobs.raw.draws_raw() {
        let (layers, report) = mesh_raw_roads(map, style);
        return RoadBuild {
            layers,
            report,
            ruts: RutLines::default(),
            tram: TramTracks::as_mapped(&map.rails),
            pieces: Vec::new(),
        };
    }
    let skeleton = road_skeleton(map, &style, &shape);
    build_roads_from(&skeleton, verge)
}

/// Общая часть сборок ступеней обочин: скелет сети, у обеих ступеней один —
/// s1 платил за подготовку сети почти столько же, сколько s0 (живой разрез
/// записи 27), а ступень меняет только обочинные куски. Всё, что финал
/// читает из карты и ручек, едет со скелетом: сборка ступени
/// ([`build_roads_from`]) берёт скелет и свою ступень — и ничего больше.
/// Ступенезависимого здесь нет: обочины, скругления обочин, лоскуты и устья
/// остаются финалу — устья читают узловой асфальт через `kerb_returns`, а
/// тот собирает и обочины (`corners/gather.rs`).
pub struct RoadSkeleton<'m> {
    map: &'m MapData,
    style: RoadStyle,
    shape: RoadShape,
    /// Подготовленная сеть — самая дорогая часть скелета.
    prepared: Drawn<'m>,
    /// Длина улицы у начала каждого way — по ней идут штрихи краски.
    stations: Vec<Station>,
    /// Островки-крошки в треугольниках узлов (`corners::small_islands`).
    islands: Vec<Vec<Vec2>>,
    /// Узлы и их краска, с островками подходов уже в разрывах: финал узлы
    /// только читает.
    junctions: Junctions,
    /// Траектории манёвров — колея в узле.
    turns: Turns,
    /// Островки на подходах к кольцам: одни и те же для узлов (здесь) и для
    /// клиньев у колец (финал).
    splitters: Vec<Splitter>,
}

/// Скелет двух сборок ступеней обочин: подготовка сети, станции краски,
/// островки узлов, узлы с краской, траектории, островки подходов. Порядок
/// внутри — порядок тех же фаз одноступенчатой сборки: траектории узлов
/// до островков подходов, узлы финализируются последними.
pub fn road_skeleton<'m>(
    map: &'m MapData,
    style: &RoadStyle,
    shape: &RoadShape,
) -> RoadSkeleton<'m> {
    let prepared = Drawn::new(map, style, shape);
    let drawn = prepared.roads();
    // узловые оси — скругления, слияния, карманы, станции штрихов, полотно
    // трамвая: их торцы стоят в точках OSM
    let nodal = prepared.axes(Axis::Nodal);
    // длина улицы у начала каждого way — по ней идут штрихи краски
    let stations = paint::street_stations(&map.network, &nodal);
    let islands = corners::small_islands(&prepared);
    // ── Узлы и скругления ──
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
    // траектории манёвров (`roads/turns.rs`) — колея в узле
    let turns = turns::Turns::new(
        &prepared,
        junctions.node_paint().junctions(),
        map.traffic_side,
    );
    // оси лент — для островков подходов; финал пересчитает их себе сам
    let ribbon = prepared.axes(Axis::Ribbon);
    // островки по правилу — на двусторонних подходах, где веера из въезда и
    // съезда в OSM нет: краска и колея подхода рвутся на их длину
    let splitters = gores::splitters::splitters(&drawn, &ribbon, prepared.rings());
    junctions.add_splitters(&splitters);
    RoadSkeleton {
        map,
        style: *style,
        shape: *shape,
        prepared,
        stations,
        islands,
        junctions,
        turns,
        splitters,
    }
}

/// Выход сборки ступеней для мира ([`build_roads_steps`]): общие слои — один
/// набор на все ступени, ступенезависимые — по ступени. Мир кладёт общие
/// один раз (`RoadLayerTag` без `LodStep` — ступень обочин их не прячет,
/// [`spawn_road_common_layers`]), ступени — с `LodStep(step)`; после
/// пересборки по стилю картина та же, а не два полных экземпляра, как у
/// слитой сборки. Слитая дверь ([`build_roads_from`]) остаётся бенчу,
/// витринам и тестам — её строки и есть контроль того, что сплит не тронул
/// геометрию.
#[derive(Default)]
pub struct RoadStepsBuild {
    /// Слои, у ступеней бит-в-бит одинаковые (отпечатки записей 28 и 32):
    /// чисто общие — из своих сборщиков, смешанные (roads, sidewalks,
    /// alleys, unpaved_roads) — скругления первой ступени с дописанной
    /// один раз общей серединой.
    pub common: Vec<LayerMesh>,
    /// Ступенезависимые слои и отчёты, в порядке ступеней.
    pub steps: Vec<StepRoadBuild>,
}

/// Ступенезависимая часть сборки ступени: обочинные слои и мощёные клинья
/// зелени, отчёт ступени и побочные выходы сборки — то же, что у
/// [`RoadBuild`], только слои свои, а не полные.
#[derive(Default)]
pub struct StepRoadBuild {
    /// `road_verges`, `road_verge_lawns`, `road_verge_yards` и хвост
    /// тротуаров-лоскутов (`sidewalk_scraps`) — всё, чем ступень расходится
    /// с другими (отпечатки записей 28 и 32).
    pub layers: Vec<LayerMesh>,
    /// Отчёт ступени **по полной картине** — свои слои плюс общие: та же
    /// сумма вершин, что у слитой сборки этой ступени, и та же строка
    /// `road meshing:`.
    pub report: RoadReport,
    /// Линии колеи: по ним строится оверлей колеи (`roads/ruts.rs`).
    pub ruts: RutLines,
    /// Трамвайные пути по нарисованной улице: их читает слой трамвая
    /// (`map/tram.rs`).
    pub tram: TramTracks,
    /// Отрисованные куски ступени — для семантического индекса.
    pub pieces: Vec<DrawnPiece>,
}

/// Сборка одной ступени обочин из общего скелета — **слитая дверь**:
/// полный набор слоёв ступени, общая середина дописана в её уклад. Выход
/// тот же, что до мирового сплита, бит-в-бит — его строки (бенч
/// [`measure_roads`], витрины, тесты) и есть контроль. Игра ходит
/// сплит-дверью ([`build_roads_steps`]): общая середина
/// ([`RoadCommon`]) считается под одну ступень.
pub fn build_roads_from(skeleton: &RoadSkeleton<'_>, verge: VergeZoomBucket) -> RoadBuild {
    let common = RoadCommon::new(skeleton, &[verge]);
    common.finish_step_merged(skeleton, 0)
}

/// Сборка нескольких ступеней обочин над одним скелетом — **сплит-дверь**:
/// общие слои один раз ([`RoadCommon::take_common_layers`]), обочинные
/// куски и лоскуты — на каждую ступень, финалы — по ядру на ступень (общее
/// только читается). Суммы вершин ступеней — те же, что у слитой сборки:
/// отчёт ступени считает полную картину (свои слои + общие).
pub fn build_roads_steps(skeleton: &RoadSkeleton<'_>, steps: &[VergeZoomBucket]) -> RoadStepsBuild {
    let mut common = RoadCommon::new(skeleton, steps);
    let CommonLayers {
        layers,
        vertices,
        paint_vertices,
    } = common.take_common_layers();
    // Уклады ступеней уходят в финалы по значению (`in_parallel_owned`):
    // клонировать сборщики ступени ради параллели — второй раз платить за её
    // геометрию (разрез записи 44: клоны 33 + scraps 3 мс ступени).
    let lays = std::mem::take(&mut common.lays);
    let steps = in_parallel_owned(lays, |lay| {
        common.finish_step_split(skeleton, lay, vertices, paint_vertices)
    });
    RoadStepsBuild {
        common: layers,
        steps,
    }
}

/// Обочинный уклад одной ступени: её скругления и обочины — в своих
/// сборщиках, вклады в лоскуты и куски — при ней. Общие фазы кладут мимо
/// ([`RoadCommon`]); финал ступени собирает слой из её кусков и общих.
struct StepLay {
    verge: VergeZoomBucket,
    kerb: corners::KerbReturns,
    /// Скругления узлов — асфальт, тротуары, проезды. У ступеней они
    /// бит-в-бит одинаковы (отпечаток записи 28), но у каждой свои: так
    /// порядок сетки лоскутов и кусков ступени остаётся её собственным.
    streets: MeshBuilder,
    alleys: MeshBuilder,
    unpaved: MeshBuilder,
    sidewalks: MeshBuilder,
    /// Обочины до отдельных тротуаров и их газоны — слои, где ступени
    /// расходятся.
    verges: MeshBuilder,
    verge_lawns: MeshBuilder,
    verge_yards: MeshBuilder,
    /// Вклады ступени в лоскуты: скругления, потом улицы по порядку заливки.
    /// Сетка по ним ленивая — к первому вопросу все вклады дописаны.
    scraps: scraps::Scraps,
    /// Отрисованные куски ступени — для семантического индекса.
    pieces: Vec<DrawnPiece>,
    /// Геометрия обочин по улицам, в порядке заливки; раскладывает лента.
    verges_by_street: Vec<verges::StreetVerges>,
}

/// Общая середина сборок ступеней: всё, что у ступеней бит-в-бит одинаково
/// (отпечаток записи 28 — асфальт, тротуары, разделительные, краска, мосты,
/// кольца, стены, переезды), считается и укладывается один раз в свои
/// сборщики. Ступенезависимое — скругления, обочины, лоскуты — живёт в
/// [`StepLay`] на ступень. Финал ([`RoadCommon::finish_step_merged`], у
/// сплит-двери — [`RoadCommon::finish_step_split`]) собирает
/// слои ступени: её куски, потом общие, потом лоскуты.
struct RoadCommon<'s> {
    lays: Vec<StepLay>,
    // ── общие сборщики: асфальт, тротуары, разделительные, краска, мосты ──
    streets: MeshBuilder,
    sidewalks: MeshBuilder,
    alleys: MeshBuilder,
    unpaved: MeshBuilder,
    median_grass: MeshBuilder,
    ring_lawns: MeshBuilder,
    ring_grass: MeshBuilder,
    wall_ribbons: MeshBuilder,
    rail_crossings: MeshBuilder,
    bridges: Bridges,
    painter: paint::Painter,
    /// Слои стоянок с островками дорог — у ступеней одинаковы (отпечаток
    /// записи 28), считаются в общей середине, финал берёт копию.
    lot_layers: lots::LotLayers,
    // ── общие значения ──
    drawn: Vec<&'s RoadLine>,
    ribbon: Vec<std::borrow::Cow<'s, [Vec2]>>,
    gores: gores::Gores,
    road_islands: islands::RoadIslands,
    ruts: RutLines,
    tram_tracks: TramTracks,
    // ── отчётные значения ──
    merge_edges: usize,
    kerb_pockets: usize,
    turning_circles: usize,
    tram_bands: usize,
    track_bridges: usize,
    level_crossings: usize,
    network_time: std::time::Duration,
    started: std::time::Instant,
}

impl<'s> RoadCommon<'s> {
    /// Общая середина сборок ступеней `steps`: скругления каждой ступени
    /// (калькуляция и укладка в её [`StepLay`]), потом общие фазы — кромки,
    /// карманы, устья, разделительные, островки, ленты, краска узлов, трамвай,
    /// переезды, стены. Читает ступень `[0]` там, где общий выход от ступени
    /// не зависит (отпечаток записи 28): у одноступенчатой сборки это её
    /// собственная ступень.
    fn new(skeleton: &'s RoadSkeleton<'s>, steps: &[VergeZoomBucket]) -> RoadCommon<'s> {
        let RoadSkeleton {
            map,
            style,
            shape,
            prepared,
            stations,
            islands,
            junctions,
            turns,
            splitters,
        } = skeleton;
        let map = *map;
        let style = *style;
        let shape = *shape;
        // ── Слои и накопители ── куда кладут общие фазы ниже: строители
        // слоёв, мосты, стоянки, краска и колея
        let started = std::time::Instant::now();
        let roads: &[RoadLine] = &map.roads;
        let mut painter = paint::Painter::new(map.traffic_side);
        let mut ruts = RutLines::default();

        let mut sidewalks = MeshBuilder::with_surface_coords();
        let mut median_grass = MeshBuilder::with_surface_coords();
        let mut alleys = MeshBuilder::with_surface_coords();
        let mut streets = MeshBuilder::with_surface_coords();
        // грунтовые улицы — своим слоем под асфальтом (`Z_UNPAVED_ROAD`)
        let mut unpaved = MeshBuilder::with_surface_coords();
        // мост — цепочка ways, и тень считается по всей цепочке; мостовые слои
        // копит он же (`roads/bridges.rs`)
        let mut bridges = Bridges::new(map);
        let mut wall_ribbons = MeshBuilder::default();
        // улицы на больших стоянках — бордюром и разметкой поверх их асфальта
        // (`roads/lots.rs`)
        let mut grounds = lots::Grounds::of(map);

        // ── Нарисованная сеть ──
        // Скелет сети — подготовка, станции, островки узлов, узлы с краской,
        // траектории, островки подходов — собран в скелете (`road_skeleton`):
        // у ступеней он общий. Общие фазы идут под ним.
        let nodes = prepared.nodes();
        let drawn = prepared.roads();
        // узловые оси — скругления, слияния, карманы, станции штрихов, полотно
        // трамвая: их торцы стоят в точках OSM
        let nodal = prepared.axes(Axis::Nodal);
        // ── Скругления ступеней ── калькуляция и укладка — в уклад ступени,
        // ступень от ступени не зависит — по ядрам: у ступеней свои сетки
        // лоскутов и свои куски. Скругления кладутся раньше всех лент своего
        // слоя: лента поверх кроет скругление, а не наоборот, и разметка
        // остаётся целой
        let meadows = Meadows::of(&map.parks, &map.grass);
        let mut lays: Vec<StepLay> = in_parallel(steps, |&verge| {
            let kerb = corners::kerb_returns(prepared, shape.corner_radius(), verge);
            let mut lay = StepLay {
                verge,
                kerb,
                streets: MeshBuilder::with_surface_coords(),
                alleys: MeshBuilder::with_surface_coords(),
                unpaved: MeshBuilder::with_surface_coords(),
                sidewalks: MeshBuilder::with_surface_coords(),
                verges: MeshBuilder::with_surface_coords(),
                verge_lawns: MeshBuilder::with_surface_coords(),
                verge_yards: MeshBuilder::with_surface_coords(),
                scraps: scraps::Scraps::default(),
                pieces: Vec::new(),
                verges_by_street: Vec::new(),
            };
            // асфальт узлов — в свои слои; порядок укладки — инвариант
            // `corners` (`KerbReturns::lay`), а не этого вызывающего
            lay.kerb.lay(
                corners::KerbLayers {
                    streets: &mut lay.streets,
                    alleys: &mut lay.alleys,
                    unpaved: &mut lay.unpaved,
                    sidewalks: &mut lay.sidewalks,
                    verges: &mut lay.verges,
                    lawns: &mut lay.verge_lawns,
                    yards: &mut lay.verge_yards,
                },
                &meadows,
                &mut lay.scraps,
                &mut lay.pieces,
            );
            lay
        });
        // общие фазы читают скругления первой ступени (`lays[0].kerb`): их
        // выход у ступеней бит-в-бит одинаков (отпечаток записи 28), у
        // одной ступени это её собственные скругления
        // ── Кромки слияний ──
        let merge_edges = merges::lay_merge_edges(
            map,
            prepared,
            &drawn,
            &nodal,
            shape.taper(),
            &mut streets,
            &mut sidewalks,
        );
        // ── Карманы у бордюра ──
        let kerb_pockets = pockets::lay_kerb_pockets(
            map,
            nodes,
            &nodal,
            junctions.row(),
            prepared,
            &mut streets,
            &mut sidewalks,
        );
        // ── Разворотные площадки ──
        let turning_circles =
            junctions::lay_turning_circles(map, nodes, prepared, &mut streets, &mut sidewalks);
        // ── Оси лент, траектории и порядок заливки ──
        // оси ленты, со стежками — до дороги, до которой OSM торец не довёл:
        // асфальт, краска, траектории, острова
        let ribbon = prepared.axes(Axis::Ribbon);
        // Широкие улицы поверх узких — см. доку модуля; ведущая узла — поверх
        // всех: её колея идёт через узел, и примыкание шире неё не должно её
        // закрыть.
        let leading = junctions.leading();
        let widths: Vec<f32> = drawn.iter().map(|road| road.width).collect();
        let order = fill_order(&widths, &leading, junctions.node_paint().junctions());
        // ── Направляющие островки, кольца и устья ──
        // направляющие островки у колец (`roads/gores.rs`) — до лент: к ним
        // дотягиваются двойные сплошные разделительных
        let gore_roads: Vec<gores::GoreRoad> = order
            .iter()
            .filter(|&&index| {
                let road = drawn[index];
                road.class == RoadClass::Street && !road.carves_navmesh()
            })
            .map(|&index| {
                gores::GoreRoad::new(
                    drawn[index],
                    &ribbon[index],
                    prepared.rings().leg_flow(index),
                )
            })
            .collect();
        // остров кольца из дуг — его замкнутая ось (`rings::Ring::path`)
        let ring_lawn_islands: Vec<&[Vec2]> = prepared
            .rings()
            .list
            .iter()
            .map(|ring| ring.path.as_slice())
            .collect();
        let ring_lawns = ring_island_lawns(&ring_lawn_islands);
        let ring_grass = ring_island_grass(&ring_lawn_islands, &map.grass);
        let mut gores = gores::Gores::of(&gore_roads, &ring_lawn_islands);
        // островки подходов — из скелета: они же в разрывах краски узлов
        gores.add_splitters(splitters);
        // клинья перед носами острых развилок улиц (`corners::fork_gore`)
        gores.add_forks(lays[0].kerb.fork_gores());
        // пустоты, зажатые проезжими частями, где ничего не замаплено (R20)
        let found_mouths = mouths(
            map,
            prepared,
            &ribbon,
            islands,
            &ring_lawn_islands,
            &gores,
            &lays[0].kerb,
        );
        // остриё устья, на которое легла зебра, отступает от неё: штриховка по
        // переходу ложилась поверх его полос (Тула, R20)
        let crossings: Vec<(Vec2, Vec2)> = junctions
            .node_paint()
            .marks()
            .zebras()
            .iter()
            .map(|zebra| (zebra.from, zebra.to))
            .collect();
        // зебра срезает только штриховку острия: асфальт устья под ней остаётся
        let hatchable_mouths = gores::mouths::clear_crossings(
            found_mouths.clone(),
            &crossings,
            node_paint::ZEBRA_LENGTH / 2.0 + MOUTH_CROSSING_GAP,
        );
        gores.add_mouths(&found_mouths, &hatchable_mouths);
        // ── Разрывы узлов и рампы слияний ──
        // три множества разрывов — каждому потребителю своё (`roads/junctions.rs`)
        let node_paint = junctions.node_paint();
        let marks = node_paint.marks();
        let asphalt = junctions.asphalt();
        let paint_breaks = junctions.paint();
        // каркасы половин у слияний сводятся в каркас продолжения
        // (`roads/merges.rs`) — по той же нарисованной оси, что и линии
        let ramps = merges::ramps(
            &prepared.merges().list,
            &drawn,
            &ribbon,
            &map.network,
            shape.taper(),
        );
        // ── Разделительные парных половин ──
        // где у пары половин кончается разделительная — осевая слияния доходит
        // до неё: торцы асфальтовой середины и носы газона, по улицам половин
        let street_of = |road: usize| map.network.street_of(road).map(|(street, _)| street);
        // разделительные парных половин (`roads/medians.rs`): асфальт — до лент
        // половин, под ними; газон с бордюром — в свой слой над тротуарами;
        // открываются по базе — у перекрёстка, кто бы его ни вёл. Двойную
        // сплошную красит не он, а мы — по его списку
        // что замыкает зазор впереди торца разделительной: проезжие части из
        // узлов её половин рядом с торцом — связка, разворот
        let closer = |halves: [usize; 2], tip: Vec2, heading: Vec2, reach: f32| {
            let mut near: Vec<usize> = halves
                .iter()
                .flat_map(|&half| nodal[half].iter())
                .filter(|point| point.distance(tip) <= 2.0 * reach && nodes.is_shared(**point))
                .flat_map(|point| nodes.roads_at(*point).iter().copied())
                .filter(|road| !halves.contains(road) && is_carriageway(drawn[*road]))
                .collect();
            near.sort_unstable();
            near.dedup();
            medians::closing_reach(
                tip,
                heading,
                reach,
                near.iter().map(|&road| ribbon[road].as_ref()),
            )
        };
        let median_drawing = medians::draw(
            prepared.pairs(),
            &medians::MedianInputs {
                base: junctions.median_base(),
                paint: paint_breaks,
                zebras: marks.zebras(),
                half: &|road| prepared.road(road).width / 2.0,
                markings: style.markings,
                pure_merge: &|at| prepared.merges().is_pure_node(at),
                reach_gores: &|midline| gores.reach(midline),
                street_of: &street_of,
                closer: &closer,
                on_bridge: &|road| drawn[road].bridge,
            },
            &mut streets,
            &mut sidewalks,
            &mut median_grass,
            bridges.fills(),
        );
        for (midline, painted, on_deck) in &median_drawing.painted {
            painter.paint_median(midline, painted, *on_deck);
        }
        // за узлом слияния, до разделительной его пары: асфальт до носа газона —
        // под лентами половин — и осевая продолжения
        streets.set_lanes(None);
        for merge in prepared.merges().list.iter().filter(|merge| merge.pure) {
            let halves = merge.halves.map(street_of);
            let ends: Vec<merges::MedianEnd> = median_drawing
                .ends
                .iter()
                .filter(|(pair, _)| {
                    pair.iter()
                        .all(|street| street.is_some() && halves.contains(street))
                })
                .map(|&(_, end)| end)
                .collect();
            for shape in merges::nose_fill(
                merge,
                &ribbon,
                &map.network,
                &ends,
                &median_drawing.lawn_kerbs,
            ) {
                push_shape(&mut streets, shape, ROAD_COLOR.to_linear());
            }
            if style.markings {
                // из торца нарисованной осевой продолжения и по её касательной
                let continuation = drawn[merge.street];
                let offset =
                    paint::axis_offset(continuation, lane_count(continuation), map.traffic_side);
                let lead = merges::LeadIn::of(merge, &ribbon, offset.unwrap_or(0.0));
                let axis = merges::merge_axis(merge, &drawn, &ribbon, &map.network, &ends, lead);
                painter.paint_merge_axis(&axis, lane_count(drawn[merge.street]));
            }
        }
        // асфальт от торца полотна до носа газона рядом — или вперёд за торец
        streets.set_lanes(None);
        for cap in median_drawing.bed_caps() {
            push_shape(&mut streets, cap, ROAD_COLOR.to_linear());
        }
        let network_time = started.elapsed();

        // ── Площади и островки до лент ──
        let (road_islands, road_bites) = islands::lay_areas(
            map,
            prepared,
            &drawn,
            &ribbon,
            islands,
            &mut streets,
            &mut sidewalks,
        );
        // ── Тротуары колец ──
        let island_insets = rings::lay_ring_island_bands(prepared, roads, style, &mut sidewalks);
        // подходы колец: их полосы тротуара копятся ниже для газона острова
        let at_ring = rings::approach_flags(prepared, drawn.len());
        let mut approach_bands: Vec<ApproachBand> = Vec::new();
        // ── Заливка колец и швы настилов ──
        // асфальт кольца — одной замкнутой заливкой по его оси, на месте первой
        // не-мостовой дуги в порядке заливки (R15): дуги, кончающиеся в узлах с
        // подходами, клались лентами с прямыми торцами, и на кривизне торцы
        // соседних дуг расходились веером — по внешней кромке клинья-щели;
        // первую дугу для вкладов ленты считает `ribbon_lane::fill_arcs`
        let seam_gaps = bridges::seam_dash_gaps(prepared, &drawn, &ribbon);
        // ── Ленты ── главный проход по улицам в порядке заливки: асфальт,
        // тротуары, обочины, краска, клинья и мосты — улица за улицей
        //
        // Обочины — по улице в потоках, на каждую ступень: улица от улицы не
        // зависит (вход — только чтение: подготовка, ленты, торцы, укусы,
        // газоны). Раскладка — ниже, в порядке заливки: порядок треугольников,
        // лоскутов и находки тот же, что у серийной укладки.
        let verges_by_street: Vec<Vec<verges::StreetVerges>> = lays
            .iter()
            .map(|lay| {
                let kerb = &lay.kerb;
                let verge = lay.verge;
                in_parallel(&order, |&index| {
                    verges::street_verge_geometry(
                        prepared,
                        &drawn,
                        &ribbon,
                        &meadows,
                        &road_bites,
                        kerb.butt(index),
                        index,
                        verge,
                    )
                })
            })
            .collect();
        for (lay, per_street) in lays.iter_mut().zip(verges_by_street) {
            lay.verges_by_street = per_street;
        }
        // Вклад кусков улиц — в потоках (`roads/ribbon_lane.rs`): улица от
        // улицы не зависит (вход — только чтение: подготовка, ленты, торцы,
        // укусы, газоны), записи идут в свои сборщики вклада. Раскладка ниже
        // дописывает вклады в порядке кусков — порядок кусков каждого слоя,
        // краски, колеи и стоянок тот же, что у серийного прохода. Куски
        // разрезаны на мостах: их сборщики (`Bridges`) общие, и ветка моста
        // остаётся серийной на своём месте в порядке заливки.
        let fill_arc = ribbon_lane::fill_arcs(prepared, &drawn, &order, &road_islands.outlines);
        let chunks = ribbon_lane::chunks(&drawn, &order);
        let states = ribbon_lane::sidewalk_states(&drawn, &order, &road_islands.outlines);
        // Состояние тротуарной полосы — улицы на начале куска, не сам кусок:
        // `zip` брал бы состояние i-й улицы для i-го куска.
        let per_chunk: Vec<_> = chunks
            .iter()
            .map(|range| (range.clone(), states[range.start]))
            .collect();
        let lane = ribbon_lane::RibbonLane {
            prepared,
            drawn: &drawn,
            order: &order,
            ribbon: &ribbon,
            ramps: &ramps,
            stations,
            junctions,
            seam_gaps: &seam_gaps,
            kerb: &lays[0].kerb,
            style,
            traffic_side: map.traffic_side,
            at_ring: &at_ring,
            outlines: &road_islands.outlines,
            grounds: &grounds,
        };
        let parts: Vec<ribbon_lane::ChunkRibbon> = in_parallel(&per_chunk, |(range, state)| {
            lane.chunk(range.clone(), *state, &fill_arc)
        });
        for (part, (range, _)) in parts.into_iter().zip(per_chunk) {
            let position = range.start;
            let index = order[position];
            let road = drawn[index];
            if road.bridge {
                // мост — кусок из одной улицы: краска и колея из вклада, тело —
                // серийно. Бордюр и тень — мосту; заливка — здесь, в порядке
                // улиц. Клин на шве земля/мост сужает настил вместе с бортиком
                // (R30); торец, севший на подход, — прямой, как бортик (R14,
                // `trimmed`). Бортика со стороны пары нет: между
                // настилами-половинами лежит разделительная в слое настилов
                // (`medians::draw`), и внутренние бортики светились бы по её
                // краям разделителем, которого на подходе нет (R30) — куски,
                // как у полосы тротуара, только по мощёной разделительной: над
                // газоном бортик и есть край моста
                if road_islands.outlines[index] {
                    continue;
                }
                let points: &[Vec2] = &ribbon[index];
                // колея гаснет по разрывам асфальта; у ведущей узла их там нет
                let breaks = asphalt.of(index);
                let lanes = road_lanes(road);
                // полный матч серийной заливки: грунтовая улица моста и
                // мощёная дорожка-мостик красятся своим цветом, не асфальтом
                let color = match road.class {
                    RoadClass::Street if road.is_unpaved_street() => UNPAVED_ROAD_COLOR,
                    RoadClass::Street => ROAD_COLOR,
                    RoadClass::Alley if road.is_paved_path() => SIDEWALK_COLOR,
                    RoadClass::Alley => ALLEY_COLOR,
                };
                ruts.lanes.extend(part.ruts);
                if let Some(painter_part) = part.painter {
                    painter.append_from(&painter_part);
                }
                let ends = prepared.taper_ends(index);
                let [head, body, tail] = if ends == [None; 2] {
                    [None, None, None]
                } else {
                    tapers::split(points, ends.map(|end| end.map(|taper| taper.length)))
                };
                let body: &[Vec2] = body.as_deref().unwrap_or(points);
                let butt = lays[0].kerb.butt(index);
                let stitched_end = prepared.stitched_end(index);
                let trimmed = [
                    head.is_some() || (butt[0] && !stitched_end[0]),
                    tail.is_some() || (butt[1] && !stitched_end[1]),
                ];
                let seam = bridges::seam_ends(&drawn, prepared.nodes(), index);
                let banded = [trimmed[0] || seam[0], trimmed[1] || seam[1]];
                let trimmed = banded;
                let wedges = tapers::wedge_run(&head, &tail, ends);
                let curb = road.curb_reach() - road.width / 2.0;
                let head_length = head.as_deref().map_or(0.0, polyline_length);
                let pieces = prepared.pairs().curb_pieces(
                    index,
                    prepared.stitch_offset(index) - head_length,
                    polyline_length(body),
                );
                let total_length = polyline_length(points);
                let curb_wedges: Vec<(&[Vec2], [[f32; 2]; 2])> = wedges
                    .iter()
                    .map(|&(path, taper, end)| {
                        let seam_half = drawn[taper.seam].width / 2.0;
                        let middle = wedge_middle(total_length, polyline_length(path), end);
                        let paired = prepared
                            .pairs()
                            .beside_paved(index, middle, 0.0)
                            .map(|left| usize::from(!left));
                        let halves = wedge_halves(taper.sides, end, |side| {
                            let curb = if paired == Some(side) { 0.0 } else { curb };
                            [seam_half + curb, road.width / 2.0 + curb]
                        });
                        (path, halves)
                    })
                    .collect();
                bridges.push_deck(
                    index,
                    points,
                    body,
                    head_length,
                    pieces.as_deref(),
                    &curb_wedges,
                    road,
                );
                let fills = bridges.fills();
                fills.set_lanes(lanes);
                push_street_fill(fills, body, road.width, color.to_linear(), breaks, trimmed);
                fills.set_lanes(lanes);
                tapers::push_asphalt(
                    fills,
                    &wedges,
                    &drawn,
                    road,
                    body,
                    breaks,
                    lanes,
                    color.to_linear(),
                    map.traffic_side,
                );
                // фартук асфальта под торцом настила-улицы на шве, на подход —
                // в слое улиц, под их краской (`bridges.rs`)
                bridges.push_seam_aprons(
                    &mut streets,
                    &wedges,
                    seam,
                    points,
                    road,
                    pieces.as_deref(),
                );
                continue;
            }
            streets.append(&part.streets);
            sidewalks.append(&part.sidewalks);
            alleys.append(&part.alleys);
            unpaved.append(&part.unpaved);
            if let Some(painter_part) = part.painter {
                painter.append_from(&painter_part);
            }
            ruts.lanes.extend(part.ruts);
            approach_bands.extend(part.approach);
            // улицы больших стоянок — вкладом; улицы внутри площадки идут в
            // порядке заливки
            grounds.accept(part.grounds);
            // обочина до отдельного тротуара — собранная по улице заранее
            // (`verges_by_street`): раскладка здесь, в порядке заливки — на
            // каждую ступень в её уклад
            for position in range {
                for lay in lays.iter_mut() {
                    lay.verges_by_street[position].apply(
                        [&mut lay.verges, &mut lay.verge_lawns, &mut lay.verge_yards],
                        &mut lay.scraps,
                        &mut lay.pieces,
                    );
                }
            }
        }
        // ── Фартуки мостиков ──
        aprons::lay_slit_aprons(
            &drawn,
            &ribbon,
            prepared,
            &mut bridges,
            &mut sidewalks,
            &mut alleys,
        );
        // ── Краска узлов и колея траекторий ──
        // (лоскуты — в финале ступени: их находка по вкладам ступени)
        // Краска читает уложенный асфальт — и ей нужен слой со скруглениями: в
        // общем сборщике их нет, они в укладах ступеней (бит-в-бит одинаковые,
        // отпечаток записи 28). Временный слой первой ступени — ровно то, что
        // читала одноступенчатая сборка; сама краска пишет в painter и колею
        let mut node_streets = lays[0].streets.clone();
        node_streets.append(&streets);
        node_paint::paint_junction_marks(
            marks,
            &node_streets,
            &mut painter,
            turns,
            &mut ruts,
            paint_breaks,
            style,
        );
        // ── Поверх асфальта: островки, трамвай, стоянки ──
        // направляющие островки у колец: асфальт — в слой улиц, поверх тротуаров,
        // разметка — в слой краски, своим мешем выше асфальта стоянок
        // (`roads/gores.rs`)
        gores.push_asphalt(&mut streets, ROAD_COLOR.to_linear());
        let (tram_tracks, tram_bands) = tram_band::lay_tram_bands(
            map,
            &drawn,
            &nodal,
            &ribbon,
            &median_drawing,
            &mut bridges,
            &mut streets,
        );
        let mut lot_layers = grounds.layers(&style, &gores, &median_drawing.paved);
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

        // ── Путепроводы и переезды ──
        // путепроводы: плита, парапет и тень — в тех же мостовых слоях; путь
        // поверх плиты кладёт `map::rail`. Трамвай на мосту едет по мосту улицы
        let mut track_bridges = 0;
        for rail in map
            .rails
            .iter()
            .filter(|rail| rail.bridge && rail.kind != RailKind::Tram)
        {
            bridges.push_track(rail);
            track_bridges += 1;
        }
        // переезды в одном уровне — по оси ленты и ширине улицы как рисуются,
        // с тротуаром там, где он рисуется (`rail::mesh_level_crossings`)
        let crossed: Vec<CrossedStreet> = (0..drawn.len())
            .filter(|&index| crossable(drawn[index]))
            .map(|index| CrossedStreet {
                axis: &ribbon[index],
                reach: [0, 1].map(|side| {
                    drawn[index].width / 2.0 + prepared.sidewalk_on(index, side).unwrap_or(0.0)
                }),
            })
            .collect();
        let (rail_crossings, level_crossings) = mesh_level_crossings(&map.rails, &crossed);
        // ── Газон островов колец ──
        rings::cover_ring_islands(prepared, &island_insets, &approach_bands, &mut median_grass);
        // ── Лента крепостной стены ──
        walls::lay_wall_ribbons(map, &mut wall_ribbons);

        RoadCommon {
            lays,
            streets,
            sidewalks,
            alleys,
            unpaved,
            median_grass,
            ring_lawns,
            ring_grass,
            wall_ribbons,
            rail_crossings,
            bridges,
            painter,
            lot_layers,
            drawn,
            ribbon,
            gores,
            road_islands,
            ruts,
            tram_tracks,
            merge_edges,
            kerb_pockets,
            turning_circles,
            tram_bands,
            track_bridges,
            level_crossings,
            network_time,
            started,
        }
    }

    /// Финал ступени `step` **слитой двери** (индекс среди ступеней
    /// [`RoadCommon::new`]): её скругления и обочины — из её уклада, общая
    /// середина — дописыванием общих сборщиков, лоскуты — находка по вкладам
    /// ступени. Порядок кусков слоя тот же, что у одноступенчатой сборки:
    /// скругления, общая середина (кромки, разделительные, ленты, краска),
    /// лоскуты. Сплит-дверь собирает финалом ступени
    /// [`RoadCommon::finish_step_split`] — только её слои.
    fn finish_step_merged(&self, skeleton: &RoadSkeleton<'_>, step: usize) -> RoadBuild {
        let RoadSkeleton {
            map,
            style,
            prepared,
            islands,
            junctions,
            turns,
            ..
        } = skeleton;
        let map = *map;
        let style = *style;
        let lay = &self.lays[step];
        // слои ступени: её скругления и обочины, дописанные общей серединой.
        // Порядок — как у одноступенчатой сборки: скругления раньше всего,
        // общие фазы между ними и лоскутами
        let mut streets = lay.streets.clone();
        streets.append(&self.streets);
        let mut sidewalks = lay.sidewalks.clone();
        sidewalks.append(&self.sidewalks);
        let mut alleys = lay.alleys.clone();
        alleys.append(&self.alleys);
        let mut unpaved = lay.unpaved.clone();
        unpaved.append(&self.unpaved);
        let mut verges = lay.verges.clone();
        let verge_lawns = lay.verge_lawns.clone();
        let verge_yards = lay.verge_yards.clone();
        let median_grass = self.median_grass.clone();
        let ring_lawns = self.ring_lawns.clone();
        let ring_grass = self.ring_grass.clone();
        let wall_ribbons = self.wall_ribbons.clone();
        let rail_crossings = self.rail_crossings.clone();
        let mut pieces = lay.pieces.clone();
        // ── Лоскуты ── находка по вкладам ступени (скругления и улицы уже в
        // списке): мощение после лент, как у одноступенчатой сборки
        let mut scraps = lay.scraps.clone();
        scraps::pave_scraps(
            &lay.kerb,
            &self.gores,
            &self.drawn,
            prepared,
            &self.ribbon,
            &self.road_islands,
            &mut scraps,
        );
        let lawn_scraps =
            scraps::lay_green_scraps(map, &mut scraps, &mut verges, &mut sidewalks, &mut pieces);
        // ── Сборка слоёв и отчёт ──
        // асфальт, тротуар и дорожка — фактурные, лента стены — плоская; три
        // мостовых слоя со своими высотами и материалами отдаёт `Bridges`
        let lot_layers = self.lot_layers.clone();
        let mut layers: Vec<LayerMesh> = [
            (
                ring_lawns,
                Z_RING_ISLAND,
                "ring_islands",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                verge_lawns,
                Z_ROAD_VERGE_LAWN,
                "road_verge_lawns",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                verge_yards,
                Z_ROAD_VERGE_YARD,
                "road_verge_yards",
                MaterialSpec::Surface(SurfaceKind::Yard),
            ),
            (
                verges,
                Z_ROAD_VERGE,
                "road_verges",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
            (
                ring_grass,
                Z_RING_GRASS,
                "ring_island_grass",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
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
                unpaved,
                Z_UNPAVED_ROAD,
                "unpaved_roads",
                MaterialSpec::Surface(SurfaceKind::Unpaved),
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
            (
                rail_crossings,
                Z_RAIL_CROSSING,
                "rail_crossings",
                MaterialSpec::Flat,
            ),
        ]
        .into_iter()
        .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
        .chain(self.bridges.clone().layers())
        .collect();
        let paint_lines = self.painter.lines;
        let paint_layers = self.painter.clone().layers();
        let paint_vertices = paint_layers
            .iter()
            .map(|layer| layer.builder.vertex_count())
            .sum();
        // что скругления насчитали — поля отчёта о них (`corners.rs`)
        let kerbs = lay.kerb.counts();
        // краска — сразу над своим асфальтом: улиц над улицами, мостов над
        // настилом; сортировка устойчива, прочие слои порядка не меняют
        layers.extend(paint_layers);
        layers.sort_by(|a, b| a.z.total_cmp(&b.z));

        let report = RoadReport {
            style,
            junctions: junctions.counts(),
            paint_lines,
            paint_vertices,
            kerb_pockets: self.kerb_pockets,
            turning_circles: self.turning_circles,
            turns: turns.maneuvers,
            arrows: if style.arrows { turns.arrows.len() } else { 0 },
            kerb_returns: kerbs.kerb_returns,
            sidewalk_returns: kerbs.sidewalk_returns,
            tile_overlaps: kerbs.tile_overlaps,
            bends: kerbs.bends,
            outer_corners: kerbs.outer_corners,
            noses: kerbs.noses,
            lawn_scraps,
            drawn: prepared.stats(),
            bridges: self.bridges.count(),
            track_bridges: self.track_bridges,
            level_crossings: self.level_crossings,
            gores: self.gores.count(),
            road_islands: [
                self.road_islands.refuges,
                self.road_islands.kerbs.len() - self.road_islands.refuges,
                self.road_islands.carriageways.len(),
                self.road_islands.walkways.len(),
            ],
            merge_edges: self.merge_edges,
            islands: islands.len(),
            tram_bands: self.tram_bands,
            vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
            network: self.network_time,
            elapsed: self.started.elapsed(),
        };
        RoadBuild {
            layers,
            report,
            ruts: self.ruts.clone(),
            tram: self.tram_tracks.clone(),
            pieces,
        }
    }

    /// Общие слои сборки **сплит-двери**: чисто общие переносятся из своих
    /// сборщиков как есть, смешанные (скругления + общая середина) —
    /// скругления первой ступени с дописанной один раз серединой, краска и
    /// мосты — один раз. Скругления ступеней бит-в-бит (отпечаток записи 28),
    /// поэтому мир рисует уклад ступени 0, а ступень 1 свои смешанные
    /// сборщики выбрасывает. Вынутые сборщики обнуляются: финалы ступеней
    /// ([`RoadCommon::finish_step_split`]) общих сборщиков не читают.
    /// Возвращает слои, их сумму и краску в ней — отчёты ступеней прибавляют
    /// счётчики к своим слоям, чтобы строка `road meshing:` осталась суммой
    /// полной картины ступени ([`RoadCommon::finish_step_merged`]).
    fn take_common_layers(&mut self) -> CommonLayers {
        // середина умирает в копии: содержимое переехало в смешанный слой,
        // и держать её до конца финалов незачем
        let streets_mid = std::mem::take(&mut self.streets);
        let sidewalks_mid = std::mem::take(&mut self.sidewalks);
        let alleys_mid = std::mem::take(&mut self.alleys);
        let unpaved_mid = std::mem::take(&mut self.unpaved);
        let lay = &mut self.lays[0];
        let mut streets = std::mem::take(&mut lay.streets);
        streets.append(&streets_mid);
        let mut sidewalks = std::mem::take(&mut lay.sidewalks);
        sidewalks.append(&sidewalks_mid);
        let mut alleys = std::mem::take(&mut lay.alleys);
        alleys.append(&alleys_mid);
        let mut unpaved = std::mem::take(&mut lay.unpaved);
        unpaved.append(&unpaved_mid);
        let median_grass = std::mem::take(&mut self.median_grass);
        let ring_lawns = std::mem::take(&mut self.ring_lawns);
        let ring_grass = std::mem::take(&mut self.ring_grass);
        let wall_ribbons = std::mem::take(&mut self.wall_ribbons);
        let rail_crossings = std::mem::take(&mut self.rail_crossings);
        let lot_layers = std::mem::take(&mut self.lot_layers);
        // мосты клонируются, а не вынимаются: отчёты ступеней читают их счёт
        let paint_layers = self.painter.clone().layers();
        let paint_vertices = paint_layers
            .iter()
            .map(|layer| layer.builder.vertex_count())
            .sum();
        let mut layers: Vec<LayerMesh> = [
            (
                ring_lawns,
                Z_RING_ISLAND,
                "ring_islands",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                ring_grass,
                Z_RING_GRASS,
                "ring_island_grass",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
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
                unpaved,
                Z_UNPAVED_ROAD,
                "unpaved_roads",
                MaterialSpec::Surface(SurfaceKind::Unpaved),
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
            (
                rail_crossings,
                Z_RAIL_CROSSING,
                "rail_crossings",
                MaterialSpec::Flat,
            ),
        ]
        .into_iter()
        .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
        .chain(self.bridges.clone().layers())
        .chain(paint_layers)
        .collect();
        // как у слитой сборки: краска — сразу над своим асфальтом; сортировка
        // устойчива, прочие слои порядка не меняют
        layers.sort_by(|a, b| a.z.total_cmp(&b.z));
        let vertices = layers.iter().map(|l| l.builder.vertex_count()).sum();
        CommonLayers {
            layers,
            vertices,
            paint_vertices,
        }
    }

    /// Финал ступени `step` **сплит-двери**: только её ступенезависимые слои
    /// — обочины из её уклада и мощёные клинья зелени отдельным слоем
    /// (`sidewalk_scraps`; слитая дверь дописывает хвост в тротуары —
    /// [`RoadCommon::finish_step_merged`], порядок кусков тот же). Отчёт —
    /// по полной картине ступени: свои слои плюс общие
    /// ([`RoadCommon::take_common_layers`]) — та же строка, что у слитой
    /// сборки.
    fn finish_step_split(
        &self,
        skeleton: &RoadSkeleton<'_>,
        lay: StepLay,
        common_vertices: usize,
        paint_vertices: usize,
    ) -> StepRoadBuild {
        let RoadSkeleton {
            map,
            style,
            prepared,
            islands,
            junctions,
            turns,
            ..
        } = skeleton;
        let map = *map;
        let style = *style;
        // Уклад ступени пришёл по значению ([`build_roads_steps`]): общие
        // сборщики ступени пусты (их содержимое у [`RoadCommon::
        // take_common_layers`]), ступенезависимое — её собственное.
        let StepLay {
            verge: _,
            kerb,
            streets: _,
            alleys: _,
            unpaved: _,
            sidewalks: _,
            mut verges,
            verge_lawns,
            verge_yards,
            mut scraps,
            mut pieces,
            verges_by_street: _,
        } = lay;
        // ── Лоскуты ── находка по вкладам ступени (скругления и улицы уже в
        // списке): мощение после лент, как у слитой сборки. Мощёные клинья
        // зелени едут в свой слой тротуарного материала: у слитой сборки они
        // лежали в тротуарах после общей середины, и слой тротуаров мира —
        // общий, ступени их не прятала бы
        scraps::pave_scraps(
            &kerb,
            &self.gores,
            &self.drawn,
            prepared,
            &self.ribbon,
            &self.road_islands,
            &mut scraps,
        );
        let mut sidewalk_scraps = MeshBuilder::with_surface_coords();
        let lawn_scraps = scraps::lay_green_scraps(
            map,
            &mut scraps,
            &mut verges,
            &mut sidewalk_scraps,
            &mut pieces,
        );
        // ── Сборка слоёв и отчёт ──
        let layers: Vec<LayerMesh> = [
            (
                verge_lawns,
                Z_ROAD_VERGE_LAWN,
                "road_verge_lawns",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                verge_yards,
                Z_ROAD_VERGE_YARD,
                "road_verge_yards",
                MaterialSpec::Surface(SurfaceKind::Yard),
            ),
            (
                verges,
                Z_ROAD_VERGE,
                "road_verges",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
            (
                sidewalk_scraps,
                Z_SIDEWALK,
                "sidewalk_scraps",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
        ]
        .into_iter()
        .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
        .collect();
        // что скругления насчитали — поля отчёта о них (`corners.rs`)
        let kerbs = kerb.counts();
        let report = RoadReport {
            style,
            junctions: junctions.counts(),
            paint_lines: self.painter.lines,
            paint_vertices,
            kerb_pockets: self.kerb_pockets,
            turning_circles: self.turning_circles,
            turns: turns.maneuvers,
            arrows: if style.arrows { turns.arrows.len() } else { 0 },
            kerb_returns: kerbs.kerb_returns,
            sidewalk_returns: kerbs.sidewalk_returns,
            tile_overlaps: kerbs.tile_overlaps,
            bends: kerbs.bends,
            outer_corners: kerbs.outer_corners,
            noses: kerbs.noses,
            lawn_scraps,
            drawn: prepared.stats(),
            bridges: self.bridges.count(),
            track_bridges: self.track_bridges,
            level_crossings: self.level_crossings,
            gores: self.gores.count(),
            road_islands: [
                self.road_islands.refuges,
                self.road_islands.kerbs.len() - self.road_islands.refuges,
                self.road_islands.carriageways.len(),
                self.road_islands.walkways.len(),
            ],
            merge_edges: self.merge_edges,
            islands: islands.len(),
            tram_bands: self.tram_bands,
            vertices: layers
                .iter()
                .map(|l| l.builder.vertex_count())
                .sum::<usize>()
                + common_vertices,
            network: self.network_time,
            elapsed: self.started.elapsed(),
        };
        StepRoadBuild {
            layers,
            report,
            ruts: self.ruts.clone(),
            tram: self.tram_tracks.clone(),
            pieces,
        }
    }
}

/// Общие слои сплит-сборки и счётчики для отчётов ступеней — выход
/// [`RoadCommon::take_common_layers`].
struct CommonLayers {
    layers: Vec<LayerMesh>,
    vertices: usize,
    paint_vertices: usize,
}

/// Дорожные слои **сырого OSM** (`RawOsm::Draw`): каждый way — простая лента
/// по своей оси OSM шириной, какую ему дал разбор (по тегу `lanes`, иначе по
/// классу), с прямыми торцами и стыками без скруглений. Ни склейки узлов, ни
/// пар половин, ни разделительных, ни устьев и скруглений бордюра, ни
/// тротуаров-полос, ни газонов обочин, ни стоянок-«больших лотов», ни
/// разметки и колеи — отдельный путь сборки, а не выключатели внутри
/// [`mesh_roads`]: его проходы переплетены, и «выключить половину» дало бы
/// картинку, которой нет ни в данных, ни в игре.
///
/// Что остаётся: слои по покрытию (асфальт улиц, грунтовые улицы, дорожки —
/// песком, мощёные — плиткой тротуара), площади дорог из данных своими
/// контурами (`area:highway`, `highway` + `area=yes`) и лента крепостной
/// стены. Узкие под широкими, как в игре. Мост — в том же слое, что улица:
/// настила, бордюра и тени у него нет.
fn mesh_raw_roads(map: &MapData, style: RoadStyle) -> (Vec<LayerMesh>, RoadReport) {
    let started = std::time::Instant::now();
    let mut streets = MeshBuilder::with_surface_coords();
    let mut unpaved = MeshBuilder::with_surface_coords();
    let mut alleys = MeshBuilder::with_surface_coords();
    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut wall_ribbons = MeshBuilder::default();

    for area in &map.road_areas {
        let (builder, color) = match area.kind {
            RoadAreaKind::Carriageway => (&mut streets, ROAD_COLOR),
            RoadAreaKind::Walkway => (&mut sidewalks, SIDEWALK_COLOR),
            RoadAreaKind::Island => (&mut sidewalks, SIDEWALK_COLOR),
        };
        builder.push_polygon(&area.outline, &[], color.to_linear());
    }
    let mut order: Vec<usize> = (0..map.roads.len()).collect();
    order.sort_by(|&a, &b| map.roads[a].width.total_cmp(&map.roads[b].width));
    for index in order {
        let road = &map.roads[index];
        let (builder, color) = match road.class {
            RoadClass::Street if road.is_unpaved_street() => (&mut unpaved, UNPAVED_ROAD_COLOR),
            RoadClass::Street => (&mut streets, ROAD_COLOR),
            RoadClass::Alley if road.is_paved_path() => (&mut sidewalks, SIDEWALK_COLOR),
            RoadClass::Alley => (&mut alleys, ALLEY_COLOR),
        };
        builder.push_ribbon(
            &road.points,
            is_ring(&road.points),
            road.width,
            color.to_linear(),
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }
    for wall in &map.walls {
        push_ribbon(
            &mut wall_ribbons,
            &wall.points,
            wall.width,
            WALL_COLOR.to_linear(),
            ROAD_JOIN,
        );
    }
    let layers: Vec<LayerMesh> = [
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
            unpaved,
            Z_UNPAVED_ROAD,
            "unpaved_roads",
            MaterialSpec::Surface(SurfaceKind::Unpaved),
        ),
        (
            streets,
            Z_ROAD,
            "roads",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (wall_ribbons, Z_WALL, "walls", MaterialSpec::Flat),
    ]
    .into_iter()
    .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
    .collect();
    let report = RoadReport {
        style,
        vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
        elapsed: started.elapsed(),
        ..Default::default()
    };
    (layers, report)
}

/// Офлайн-замер дорожных слоёв — строками `LayerCost`, как у зданий и машин.
///
/// Своей сборки у него нет: он зовёт тот же [`mesh_roads`], что и игра. До шва
/// дорожные слои мерились только строкой `road meshing:` из живого приложения,
/// то есть ровно тем способом, который на macOS врёт (App Nap).
pub fn measure_roads(map: &MapData) -> Vec<LayerCost> {
    let (layers, report) = mesh_roads(
        map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
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
/// `VergeZoomBucket` в списке **нет**: ступень обочины больше не пересобирает
/// слои — обе ступени лежат в мире заранее и пересечение порога переключает
/// им видимость (`show_road_steps`). Остальные три причины собирают **текущую**
/// ступень; достройка второй падает на первое пересечение порога после
/// пересборки (`show_road_steps`) — раз на смену стиля, как раньше было на
/// каждое пересечение.
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

/// Пересборка дорожных слоёв по стилю, форме или солнцу: деспавн всех слоёв
/// (общие тоже — они несут `RoadLayerTag`) и спавн общего набора и текущей
/// ступени сплит-дверью ([`build_roads_steps`]); вторая после этого считается
/// недостроенной и достраивается `show_road_steps` на первом пересечении
/// порога — только ступенезависимыми слоями, общие уже стоят.
// входы — те же ресурсы, что читает сборка; сводить их в тип ради линта нечего
#[allow(clippy::too_many_arguments)]
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    style: Res<RoadStyle>,
    shape: Res<RoadShapeOnMap>,
    verge: Res<VergeZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<RoadLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let skeleton = road_skeleton(&map, &style, &shape.0);
    let RoadStepsBuild { common, steps } = build_roads_steps(&skeleton, &[*verge]);
    let StepRoadBuild {
        layers,
        report,
        ruts,
        tram,
        pieces,
    } = steps.into_iter().next().expect("одна ступень — один выход");
    // линии колеи — ресурсом: по нему строится оверлей колеи; пути трамвая —
    // по ним пересобирается слой трамвая; отрисованные куски — по ним
    // отвечает семантический индекс
    commands.insert_resource(ruts);
    commands.insert_resource(tram);
    commands.insert_resource(DrawnPieces(pieces));
    spawn_road_common_layers(&mut commands, &mut meshes, &materials, common);
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        (layers, report),
        verge.index,
        true,
        false,
    );
}

/// Видимость дорожных слоёв по ступени обочины, каждый кадр, идемпотентно
/// (`zoom::LodStep`): смена ступени стоит переключение `Visibility`, а не
/// пересборку. Краску система не трогает (`Without<PaintTag>`): у неё нет
/// ступеней обочины, её вид ведёт `paint::show_paint`.
///
/// Вторая половина — достройка: после пересборки по стилю/солнцу
/// (`rebuild_roads`) в мире живёт только текущая ступень, и первое же
/// пересечение порога собирает недостающую здесь же — раз на смену стиля, как
/// раньше было на каждое пересечение. Краску достройка не переспавнивает
/// (`spawn_road_meshes`): она уже стоит от пересборки. На входе в мир обе
/// ступени уже собраны (потоком загрузки, `map/prepare.rs`), так что в
/// норме система ничего не строит. Побочным выходом сборки тут пренебрегаем:
/// линии колеи и пути трамвая от ступени обочины не зависят, а ресурсами их
/// записала сборка текущей ступени.
#[allow(clippy::too_many_arguments)]
pub fn show_road_steps(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    verge: Res<VergeZoomBucket>,
    style: Res<RoadStyle>,
    shape: Res<RoadShapeOnMap>,
    map: Res<MapData>,
    mut present: Local<Vec<bool>>,
    mut layers: Query<(&LodStep, &mut Visibility), (With<RoadLayerTag>, Without<paint::PaintTag>)>,
) {
    let steps = VergeLods::max_zooms().count();
    // повторно используемый буфер: система работает каждый кадр, и новая
    // аллокация на каждом запуске была бы платой из чеклиста производительности
    present.clear();
    present.resize(steps, false);
    for (step, mut visibility) in &mut layers {
        if step.0 < steps {
            present[step.0] = true;
        }
        visibility.set_if_neq(if step.0 == verge.index {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
    // пересечение порога — единственный случай, когда ступень может
    // отсутствовать; на кадрах без пересечения система только красит
    if !verge.is_changed() {
        return;
    }
    for (step, &was_present) in present.iter().enumerate() {
        if was_present {
            continue;
        }
        // достройка — сплит-дверью: общие слои уже стоят в мире (от
        // пересборки или входа), общий набор сборки выбрасывается, спавнятся
        // только ступенезависимые. Общая середина при этом строится зря
        // (~1 с, раз на смену стиля) — как и слитая дверь строила полный шаг
        // зря; узкая дверь без общих фаз усложнила бы `RoadCommon`
        let skeleton = road_skeleton(&map, &style, &shape.0);
        let mut build = build_roads_steps(&skeleton, &[VergeZoomBucket::at(step)]);
        let StepRoadBuild { layers, report, .. } = build.steps.remove(0);
        spawn_road_meshes(
            &mut commands,
            &mut meshes,
            &materials,
            (layers, report),
            step,
            step == verge.index,
            false,
        );
    }
}

/// Положить в мир **общие слои** дороги сплит-сборки
/// ([`build_roads_steps`]): один экземпляр на город, `RoadLayerTag` **без
/// `LodStep`** — ступень обочин их не прячет (`show_road_steps` читает
/// `LodStep`). Краска несёт свой `PaintTag` — её вид ведёт
/// `paint::show_paint`, как и у слитой сборки. Дверь нужна отдельная именно
/// из-за отсутствия метки: `spawn_road_meshes` всегда ставит `LodStep(step)`.
pub fn spawn_road_common_layers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    layers: Vec<LayerMesh>,
) {
    // меши краски несут ещё и свой вид — по нему их прячет ступень зума
    // (`paint::show_paint`)
    let (paint, rest): (Vec<LayerMesh>, Vec<LayerMesh>) = layers
        .into_iter()
        .partition(|layer| paint::PaintTag::of(layer.name).is_some());
    spawn_layers(
        commands,
        meshes,
        materials,
        rest.into_iter()
            .flat_map(|layer| layer.into_chunks(surface::MAP_CHUNK)),
        (RoadLayerTag, Visibility::Visible),
    );
    for layer in paint {
        let Some(tag) = paint::PaintTag::of(layer.name) else {
            continue;
        };
        spawn_layers(commands, meshes, materials, [layer], (RoadLayerTag, tag));
    }
}

/// Положить в мир ступенезависимые слои сборки: под `RoadLayerTag` с
/// `zoom::LodStep(step)`, плюс отчёт в лог. Дверь ступеней сплит-сборки
/// ([`spawn_map`], достройка [`show_road_steps`], пересборка
/// [`rebuild_roads`]) и слитой сборки витрин — форма
/// `buildings::spawn_building_meshes`: дверь нужна не по числу меток, а по
/// числу вызывающих. Общие слои спавнит [`spawn_road_common_layers`] —
/// без `LodStep`, ступень их не прячет.
///
/// `step` — ступень обочины, под которую собраны слои: обе ступени лежат в
/// мире сразу (собранные потоком загрузки, `map/prepare.rs`), скрытые, и
/// пересечение порога переключает видимость (`show_road_steps`), а не
/// пересобирает дорогу. `visible` — виден ли этот шаг при спавне.
///
/// `spawn_paint` — спавнить ли краску (`PaintTag`): её геометрия от ступени
/// обочины не зависит (в отчётах обеих ступеней одни и те же 6121 линия и
/// 280865 вершин), и второй экземпляр дал бы двойную покраску и лишние
/// вершины. В игре краска едет в общих слоях ([`spawn_road_common_layers`]),
/// так что ступеням флага не надо; `true` остался слитой сборке витрины,
/// которая краску в своих слоях несёт.
pub fn spawn_road_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    (layers, report): (Vec<LayerMesh>, RoadReport),
    step: usize,
    visible: bool,
    spawn_paint: bool,
) {
    // меши краски несут ещё и свой вид — по нему их прячет ступень зума
    // (`paint::show_paint`)
    let (paint, rest): (Vec<LayerMesh>, Vec<LayerMesh>) = layers
        .into_iter()
        .partition(|layer| paint::PaintTag::of(layer.name).is_some());
    let visibility = if visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    spawn_layers(
        commands,
        meshes,
        materials,
        // непрозрачные слои режутся на куски карты (`LayerMesh::into_chunks`
        // сам не трогает полупрозрачные тени и краску): каждая запись — тот
        // же слой под тем же именем, метка с LodStep ложится на каждый кусок,
        // и `show_road_steps` перекрашивает их как один
        rest.into_iter()
            .flat_map(|layer| layer.into_chunks(surface::MAP_CHUNK)),
        (RoadLayerTag, LodStep(step), visibility),
    );
    if spawn_paint {
        for layer in paint {
            let Some(tag) = paint::PaintTag::of(layer.name) else {
                continue;
            };
            spawn_layers(commands, meshes, materials, [layer], (RoadLayerTag, tag));
        }
    }
    info!("{report}");
}

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин обязан
/// рваться на тех же перекрёстках, на которых рвётся разметка, и второго
/// восстановления узлов по общим нодам заводить незачем.
pub(super) mod junctions;

mod aprons;
/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин стоит на той
/// же оси, что и лента.
pub(super) mod axis;
mod bridges;
mod corners;
mod drawn;
mod footbridge;
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
/// Вклад одной улицы главного прохода ленты — потокам; раскладка в
/// [`RoadCommon::new`] дописывает вклады в порядке заливки.
mod ribbon_lane;
mod ribbons;
/// Открыт для разбора: Y-подход с хвостом чужих улиц выпрямляется в данных
/// (`rings::straighten_tails`), до сечений, — по тем же кольцам и ногам,
/// что потом рисует лента.
pub(crate) mod rings;
/// Линии колеи — оверлей вкладки Debug (`ui/debug/overlays.rs`).
pub mod ruts;
mod scraps;
/// Открыт наружу для панели и витрины: ресурс ручек формы и глобаль ширины
/// полосы.
pub mod shape;
/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин прерывается
/// на клине, где бордюр ближе к оси, чем полуширина участка.
pub(super) mod tapers;
mod tram_band;
/// Открыт для слоя трамвая (`map/tram.rs`): ресурс путей по нарисованной
/// улице.
pub mod tram_lay;
mod turns;
mod verges;
mod walls;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;
