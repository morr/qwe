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
use self::islands::push_street_fill;
pub use self::junctions::JunctionCounts;
#[cfg(test)]
use self::junctions::turning_radius;
use self::junctions::{MOUTH_CROSSING_GAP, mouths};
pub use self::node_paint::CrossingMode;
pub use self::ribbons::push_ribbon;
use self::ribbons::{push_sidewalk_to_seams, wedge_halves, wedge_middle};
use self::rings::{ApproachBand, ring_island_grass, ring_island_lawns};
use self::ruts::{LaneRuts, RutLines};
pub use self::shape::{ROAD_JOIN, RoadJoin, RoadStyle};
use self::shape::{ROAD_RIBBON, RoadShape, RoadShapeOnMap};
use self::tram_lay::TramTracks;
pub(crate) use self::verges::paved_verge;
use self::verges::push_verges;
pub use self::verges::{VergeLods, VergeZoomBucket};
#[cfg(test)]
use self::walls::Fortresses;
use self::walls::Meadows;
pub(crate) use self::walls::Z_WALL;
use crate::map::SunOnMap;
use crate::map::meshing::{Break, LaneFrame, MeshBuilder, RibbonCap, RibbonJoin};
#[cfg(test)]
use crate::map::osm::model::RoadNodeKind;
use crate::map::osm::model::{RoadAreaKind, polyline_length};
use crate::map::osm::{MapData, RailKind, RoadClass, RoadLine};
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
    let drawn = Drawn::new(map, &style, &shape, VergeZoomBucket::for_zoom(0.0));
    let returns = corners::kerb_returns(&drawn, shape.corner_radius());
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
    // ── Слои и накопители ── куда кладут фазы ниже: строители слоёв,
    // мосты, стоянки, краска и колея
    let started = std::time::Instant::now();
    let roads: &[RoadLine] = &map.roads;
    let mut painter = paint::Painter::new(map.traffic_side);
    let mut ruts = RutLines::default();

    let mut sidewalks = MeshBuilder::with_surface_coords();
    // обочины до отдельных тротуаров — под зеленью (`Z_ROAD_VERGE`)
    let mut verges = MeshBuilder::with_surface_coords();
    // и газон широких обочин — под их плиткой: травой двора
    // (`Z_ROAD_VERGE_YARD`), а у замапленного газона — его лугом
    // (`Z_ROAD_VERGE_LAWN`, `Meadows`)
    let mut verge_lawns = MeshBuilder::with_surface_coords();
    let mut verge_yards = MeshBuilder::with_surface_coords();
    let meadows = Meadows::of(&map.parks, &map.grass);
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
    // Дороги так, как они рисуются (`roads/drawn.rs`): переезд через тротуар —
    // асфальтом проезда, а не песочной дорожкой, дуга кольца — сечением всего
    // кольца; узлы, оси улиц, стежки, клинья и слияния — там же. Ступень зума
    // обочин — с собой: «какие обочины рисуются» — вопрос той же подготовки
    let prepared = Drawn::new(map, &style, &shape, verge);
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
    // газоны обочин и мощение вокруг них — для лоскутов (`roads/scraps.rs`):
    // угол газона — газон, мощение собирается после лент
    let mut scraps = scraps::Scraps::default();
    // отрисованные куски — тем же проходом: каждый логический кусок обочины,
    // лоскут и узловой кусок запоминается для семантического индекса
    let mut pieces: Vec<DrawnPiece> = Vec::new();
    // асфальт узлов — в свои слои; порядок укладки — инвариант `corners`
    // (`KerbReturns::lay`), а не этого вызывающего
    kerb_returns.lay(
        corners::KerbLayers {
            streets: &mut streets,
            alleys: &mut alleys,
            unpaved: &mut unpaved,
            sidewalks: &mut sidewalks,
            verges: &mut verges,
            lawns: &mut verge_lawns,
            yards: &mut verge_yards,
        },
        &meadows,
        &mut scraps,
        &mut pieces,
    );
    // ── Кромки слияний ──
    let merge_edges = merges::lay_merge_edges(
        map,
        &prepared,
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
        &prepared,
        &mut streets,
        &mut sidewalks,
    );
    // ── Разворотные площадки ──
    let turning_circles =
        junctions::lay_turning_circles(map, nodes, &prepared, &mut streets, &mut sidewalks);
    // ── Оси лент, траектории и порядок заливки ──
    // оси ленты, со стежками — до дороги, до которой OSM торец не довёл:
    // асфальт, краска, траектории, острова
    let ribbon = prepared.axes(Axis::Ribbon);
    // траектории манёвров (`roads/turns.rs`) — колея в узле
    let turns = turns::Turns::new(
        &prepared,
        junctions.node_paint().junctions(),
        map.traffic_side,
    );
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
    // островки по правилу — на двусторонних подходах, где веера из въезда и
    // съезда в OSM нет: краска и колея подхода рвутся на их длину
    let splitters = gores::splitters::splitters(&drawn, &ribbon, prepared.rings());
    junctions.add_splitters(&splitters);
    gores.add_splitters(&splitters);
    // клинья перед носами острых развилок улиц (`corners::fork_gore`)
    gores.add_forks(kerb_returns.fork_gores());
    // пустоты, зажатые проезжими частями, где ничего не замаплено (R20)
    let found_mouths = mouths(
        map,
        &prepared,
        &ribbon,
        &islands,
        &ring_lawn_islands,
        &gores,
        &kerb_returns,
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
    let mut median_grass = MeshBuilder::with_surface_coords();
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
        &prepared,
        &drawn,
        &ribbon,
        &islands,
        &mut streets,
        &mut sidewalks,
    );
    // ── Тротуары колец ──
    let island_insets = rings::lay_ring_island_bands(&prepared, roads, style, &mut sidewalks);
    // подходы колец: их полосы тротуара копятся ниже для газона острова
    let at_ring = rings::approach_flags(&prepared, drawn.len());
    let mut approach_bands: Vec<ApproachBand> = Vec::new();
    // ── Заливка колец и швы настилов ──
    // асфальт кольца — одной замкнутой заливкой по его оси, на месте первой
    // его дуги в порядке заливки (R15): дуги, кончающиеся в узлах с
    // подходами, клались лентами с прямыми торцами, и на кривизне торцы
    // соседних дуг расходились веером — по внешней кромке клинья-щели
    let mut ring_filled = vec![false; prepared.rings().list.len()];
    let seam_gaps = bridges::seam_dash_gaps(&prepared, &drawn, &ribbon);
    // ── Ленты ── главный проход по улицам в порядке заливки: асфальт,
    // тротуары, обочины, краска, клинья и мосты — улица за улицей
    for index in order {
        let road = drawn[index];
        // замкнутая линия площади (`highway=*` + `area=yes`) — контур
        // заливки выше, а не кольцо ленты
        if road_islands.outlines[index] {
            continue;
        }
        let ring_index = prepared.rings().index_of(index);
        // мощёная дорожка — плиткой тротуара и в его слое (`paved_path`)
        let paved_path = road.is_paved_path();
        let unpaved_street = road.is_unpaved_street();
        let color = match road.class {
            RoadClass::Street if unpaved_street => UNPAVED_ROAD_COLOR,
            RoadClass::Street => ROAD_COLOR,
            RoadClass::Alley if paved_path => SIDEWALK_COLOR,
            RoadClass::Alley => ALLEY_COLOR,
        };
        let points: &[Vec2] = &ribbon[index];
        // колея гаснет по разрывам асфальта; у ведущей узла их там нет
        let breaks = asphalt.of(index);
        let lanes = road_lanes(road);
        // колея полос ляжет по этой оси и этой раскладке — её и запомнить
        // для оверлея (`roads/ruts.rs`); у кольца — по его замкнутой оси
        if let Some(frame) = lanes {
            let axis = match ring_index {
                Some(ring) if ring_filled[ring] => None,
                Some(ring) => Some(prepared.rings().list[ring].path.clone()),
                None => Some(points.to_vec()),
            };
            if let Some(axis) = axis {
                ruts.lanes.push(LaneRuts { axis, frame });
            }
        }
        // линии краски — по той же оси, разрывам и клиньям, что и асфальт
        if style.markings {
            let wedges =
                paint::wedge_ends(points, prepared.tapers(), &drawn, index, map.traffic_side);
            // у половины слияния на шве — тихий разрыв в узле (S3)
            let base = paint_breaks.of(index);
            let (cut, quiet): (Vec<Break>, Vec<Break>);
            let line_breaks = if seam_gaps[index].is_empty() {
                base
            } else {
                cut = [base.cut, &seam_gaps[index]].concat();
                quiet = [base.quiet, &seam_gaps[index]].concat();
                paint::LineBreaks {
                    cut: &cut,
                    quiet: &quiet,
                    ..base
                }
            };
            painter.paint(
                road,
                points,
                line_breaks,
                wedges,
                node_paint.pockets().of(index),
                ramps[index],
                stations[index],
            );
        }
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
        // торец на шве земля/мост (`bridges::seam_ends`) — прямой, как бортик
        // настила: у настила — всё (R14), у подхода — полоса тротуара и
        // обочина, чей полудиск светился по сторонам настила (R32). Асфальт
        // подхода остаётся круглым: его торец под настилом, а на изломе шва
        // закрывает щель между прямыми торцами
        let seam = bridges::seam_ends(&drawn, prepared.nodes(), index);
        let banded = [trimmed[0] || seam[0], trimmed[1] || seam[1]];
        let trimmed = if road.bridge { banded } else { trimmed };
        let butt = [butt[0] || seam[0], butt[1] || seam[1]];
        let wedges = tapers::wedge_run(&head, &tail, ends);
        if road.bridge {
            // бордюр и тень — мосту; заливка — здесь, в порядке улиц. Клин на
            // шве земля/мост сужает настил вместе с бортиком (R30); торец,
            // севший на подход, — прямой, как бортик (R14, `trimmed`).
            // Бортика со стороны пары нет: между настилами-половинами лежит
            // разделительная в слое настилов (`medians::draw`), и внутренние
            // бортики светились бы по её краям разделителем, которого на
            // подходе нет (R30) — куски, как у полосы тротуара, только по
            // мощёной разделительной: над газоном бортик и есть край моста
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
            bridges.push_seam_aprons(&mut streets, &wedges, seam, points, road, pieces.as_deref());
            continue;
        }

        // тротуар кольца — одной лентой на всё кольцо (`push_ring_edges`)
        let ring = prepared.rings().of(index);
        if let Some(sidewalk) = prepared.sidewalk_drawn(index).filter(|_| ring.is_none()) {
            let band = |road: &RoadLine, sidewalk: f32| road.width + 2.0 * sidewalk;
            // у половины разделённой улицы тротуара со стороны пары нет;
            // тело клиновой половины начинается за головным клином — куски
            // пары сдвигаются на его длину. На самом клине тротуар как был
            // (ниже): там его кроет газон или его бордюр
            let (sides, total) = (road.sidewalk().sides(), polyline_length(body));
            let head_length = head.as_deref().map_or(0.0, polyline_length);
            let stitch = prepared.stitch_offset(index) - head_length;
            let pieces = prepared.pairs().band_pieces(index, sides, stitch, total);
            // на шве земля/мост полоса сходит клином в бортик настила; торец
            // под клином сечения — не шов тела
            let curb_reach = bridges::seam_curb_reach(&drawn, prepared.nodes(), index);
            let seam_reach = [
                curb_reach[0].filter(|_| head.is_none()),
                curb_reach[1].filter(|_| tail.is_none()),
            ];
            push_sidewalk_to_seams(
                &mut sidewalks,
                body,
                [road.width, sidewalk],
                pieces.as_deref(),
                SIDEWALK_COLOR.to_linear(),
                banded,
                seam_reach,
            );
            if at_ring[index] {
                approach_bands.push(ApproachBand {
                    axis: body.to_vec(),
                    width: band(road, sidewalk),
                    butt: banded,
                });
            }
            // клин тротуара — по сторонам, как у асфальта (`roads/tapers.rs`)
            tapers::push_sidewalks(
                &mut sidewalks,
                &wedges,
                &drawn,
                &prepared,
                index,
                points,
                road,
                body,
                sidewalk,
                SIDEWALK_COLOR.to_linear(),
            );
        }
        // обочина до отдельного тротуара — лентой от оси за кромку на её
        // ширину, в свою сторону: проезжую часть она кроет под асфальтом, а
        // до угла узла доходит торцом, и угол между двумя такими улицами
        // замощён их обочинами
        let verged = verges::street_verged(&prepared, index, ring.map(|ring| ring.ccw));
        let bites = road_bites.on_points(index, points, verged[0] > 0.0 || verged[1] > 0.0);
        push_verges(
            [&mut verges, &mut verge_lawns, &mut verge_yards],
            &meadows,
            index as u32,
            road,
            points,
            butt,
            &bites,
            verged,
            &mut scraps,
            &mut pieces,
        );
        // слой заливки берётся после полосы тротуара: мощёная дорожка
        // ложится в тот же слой, а полоса выше брала его сама
        let fill = match road.class {
            RoadClass::Street if unpaved_street => &mut unpaved,
            RoadClass::Street => &mut streets,
            RoadClass::Alley if paved_path => &mut sidewalks,
            RoadClass::Alley => &mut alleys,
        };
        // Кромка клина половины разделённой улицы со стороны пары прямая
        // сама: разводка пары ставит ось клина по его суженной полуширине
        // (`Pairs::align`), и сужается одна внешняя кромка.
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
        // асфальт, упёршийся в грунтовку, — до её кромки (`corners.rs`)
        let setback = kerb_returns.setback(index);
        let set_back: Option<Vec<Vec2>> = (setback != [0.0; 2] && head.is_none() && tail.is_none())
            .then(|| tapers::cut(body, setback[0], polyline_length(body) - setback[1]))
            .filter(|cut| cut.len() >= 2);
        let fill_body = set_back.as_deref().unwrap_or(body);
        match ring_index {
            Some(ring) if ring_filled[ring] => {}
            Some(ring) => {
                ring_filled[ring] = true;
                let ring = &prepared.rings().list[ring];
                let breaks: Vec<Break> = ring
                    .roads
                    .iter()
                    .flat_map(|&arc| asphalt.of(arc).iter().copied())
                    .collect();
                push_street_fill(
                    fill,
                    &ring.path,
                    ring.width,
                    color.to_linear(),
                    &breaks,
                    [false; 2],
                );
            }
            None => push_street_fill(
                fill,
                fill_body,
                road.width,
                color.to_linear(),
                breaks,
                trimmed,
            ),
        }
        fill.set_lanes(lanes);
        tapers::push_asphalt(
            fill,
            &wedges,
            &drawn,
            road,
            body,
            breaks,
            lanes,
            color.to_linear(),
            map.traffic_side,
        );
        // дорожка на мостик расширяется в него клином (S5)
        footbridge::lay_flares(fill, &drawn, &prepared, road, points, index, color);
        if road.class == RoadClass::Street && !road.passage {
            grounds.push(road, points);
        }
    }
    // ── Фартуки мостиков ──
    aprons::lay_slit_aprons(
        &drawn,
        &ribbon,
        &prepared,
        &mut bridges,
        &mut sidewalks,
        &mut alleys,
    );
    // ── Лоскуты ──
    scraps::pave_scraps(
        &kerb_returns,
        &gores,
        &drawn,
        &prepared,
        &ribbon,
        &road_islands,
        &mut scraps,
    );
    let lawn_scraps =
        scraps::lay_green_scraps(map, &mut scraps, &mut verges, &mut sidewalks, &mut pieces);
    // ── Краска узлов и колея траекторий ──
    node_paint::paint_junction_marks(
        marks,
        &streets,
        &mut painter,
        &turns,
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
    let bridge_count = bridges.count();
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
    rings::cover_ring_islands(
        &prepared,
        &island_insets,
        &approach_bands,
        &mut median_grass,
    );
    // ── Лента крепостной стены ──
    walls::lay_wall_ribbons(map, &mut wall_ribbons);

    // ── Тела вынесенных фаз ── непрерывные куски того же прохода, в порядке
    // вызова; их имена и ход — в баннерах наверху

    // ── Сборка слоёв и отчёт ──
    // асфальт, тротуар и дорожка — фактурные, лента стены — плоская; три
    // мостовых слоя со своими высотами и материалами отдаёт `Bridges`
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
    .chain(bridges.layers())
    .collect();
    let paint_lines = painter.lines;
    let paint_layers = painter.layers();
    let paint_vertices = paint_layers
        .iter()
        .map(|layer| layer.builder.vertex_count())
        .sum();
    // что скругления насчитали — поля отчёта о них (`corners.rs`)
    let kerbs = kerb_returns.counts();
    // краска — сразу над своим асфальтом: улиц над улицами, мостов над
    // настилом; сортировка устойчива, прочие слои порядка не меняют
    layers.extend(paint_layers);
    layers.sort_by(|a, b| a.z.total_cmp(&b.z));

    let report = RoadReport {
        style,
        junctions: junctions.counts(),
        paint_lines,
        paint_vertices,
        kerb_pockets,
        turning_circles,
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
        bridges: bridge_count,
        track_bridges,
        level_crossings,
        gores: gores.count(),
        road_islands: [
            road_islands.refuges,
            road_islands.kerbs.len() - road_islands.refuges,
            road_islands.carriageways.len(),
            road_islands.walkways.len(),
        ],
        merge_edges,
        islands: islands.len(),
        tram_bands,
        vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
        network: network_time,
        elapsed: started.elapsed(),
    };
    RoadBuild {
        layers,
        report,
        ruts,
        tram: tram_tracks,
        pieces,
    }
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
/// (обе ступени — вторая после этого считается недостроенной и достраивается
/// `show_road_steps` на первом пересечении порога) и спавн текущей ступени.
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
    let RoadBuild {
        layers,
        report,
        ruts,
        tram,
        pieces,
    } = build_roads(&map, *style, shape.0, *verge);
    // линии колеи — ресурсом: по нему строится оверлей колеи; пути трамвая —
    // по ним пересобирается слой трамвая; отрисованные куски — по ним
    // отвечает семантический индекс
    commands.insert_resource(ruts);
    commands.insert_resource(tram);
    commands.insert_resource(DrawnPieces(pieces));
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        (layers, report),
        verge.index,
        true,
        true,
    );
}

/// Достроить на входе в мир ступень обочины, которую `spawn_map` не собирал:
/// обе лежат в мире заранее (`RoadLayerTag` + `zoom::LodStep`), и пересечение
/// порога зума переключает им видимость (`show_road_steps`), а не
/// пересобирает дорогу — сборка текущей ступени стоила кадру 380–480 мс.
///
/// Ступень дана вычитанием, а не запросом: команды `spawn_map` применятся
/// после этой системы, и ни одна сущность слоёв в запрос ещё не видна. Таблица
/// `VergeLods` состоит из двух ступеней — ближней и дальней, — так что не
/// собранная ровно одна.
pub fn build_other_road_step(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    verge: Res<VergeZoomBucket>,
    style: Res<RoadStyle>,
    shape: Res<RoadShapeOnMap>,
    map: Res<MapData>,
) {
    let steps = VergeLods::max_zooms().count();
    for step in 0..steps {
        if step == verge.index {
            continue;
        }
        let RoadBuild { layers, report, .. } =
            build_roads(&map, *style, shape.0, VergeZoomBucket::at(step));
        spawn_road_meshes(
            &mut commands,
            &mut meshes,
            &materials,
            (layers, report),
            step,
            false,
            false,
        );
    }
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
/// ступени уже собраны (`spawn_map` + `build_other_road_step`), так что в
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
        let RoadBuild { layers, report, .. } =
            build_roads(&map, *style, shape.0, VergeZoomBucket::at(step));
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

/// Положить в мир то, что собрал [`mesh_roads`]: слои под `RoadLayerTag`,
/// плюс отчёт в лог. Одна дверь для `rebuild_roads` и `spawn_map` — форма
/// `buildings::spawn_building_meshes`: дверь нужна не по числу меток, а по
/// числу вызывающих.
///
/// `step` — ступень обочины, под которую собраны слои: обе ступени лежат в
/// мире сразу (`build_other_road_step` достраивает не текущую), скрытые, и
/// пересечение порога переключает видимость (`show_road_steps`), а не
/// пересобирает дорогу. `visible` — виден ли этот шаг при спавне.
///
/// `spawn_paint` — спавнить ли краску (`PaintTag`): её геометрия от ступени
/// обочины не зависит (в отчётах обеих ступеней одни и те же 6121 линия и
/// 280865 вершин), и второй экземпляр дал бы двойную покраску и лишние
/// вершины. Краску спавнят только сборки, которые отвечают за неё в мире, —
/// вход в мир и полная пересборка; достройка недостающей ступени
/// (`show_road_steps`) нет: краска уже стоит от пересборки по стилю.
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
        rest,
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
