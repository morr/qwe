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
//!
//! Предметы вынесены в субмодули: `roads/build.rs` — сборка области
//! ([`build_roads`], скелет, сплит- и слитая двери, отрисованные куски),
//! `roads/common.rs` — общая середина ступеней (`RoadCommon`), `roads/raw.rs`
//! — сырой путь OSM и слитая дверь [`mesh_roads`]; здесь — классификация,
//! [`RoadReport`] и спавн-системы.

use bevy::prelude::*;

pub use self::bridges::BridgeReport;
pub use self::build::{
    DrawnPiece, DrawnPieces, PieceKind, RoadBuild, RoadStepsBuild, StepRoadBuild, build_roads,
    build_roads_steps, road_skeleton,
};
pub use self::drawn::{Axis, Drawn, DrawnStats};
pub use self::junctions::JunctionCounts;
#[cfg(test)]
use self::junctions::turning_radius;
pub use self::node_paint::CrossingMode;
pub use self::raw::mesh_roads;
pub use self::ribbons::push_ribbon;
pub use self::shape::{ROAD_JOIN, RoadJoin, RoadStyle};
use self::shape::{ROAD_RIBBON, RoadShape, RoadShapeOnMap};
pub(crate) use self::verges::paved_verge;
pub use self::verges::{VergeLods, VergeZoomBucket};
#[cfg(test)]
use self::walls::Fortresses;
use self::walls::Meadows;
use crate::map::SunOnMap;
use crate::map::meshing::LaneFrame;
#[cfg(test)]
use crate::map::meshing::MeshBuilder;
#[cfg(test)]
use crate::map::osm::model::RoadNodeKind;
use crate::map::osm::{MapData, RoadClass, RoadLine};
use crate::map::surface::{self, LayerCost, LayerMaterials, LayerMesh, spawn_layers};
#[cfg(test)]
use crate::map::surface::{MaterialSpec, SurfaceKind};
use crate::map::zoom::{LodStep, ZoomLods};
use crate::prefs::retuned;

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

/// Цвет заливки ленты дороги её классом — закон серийной заливки одним
/// матчем: грунтовая улица и мощёная дорожка красятся своим цветом, не
/// асфальтом. Читают его лента улицы (`roads/ribbon_lane.rs`) и лента моста
/// в общей сборке — их копии держались комментарием «полный матч».
fn fill_color(road: &RoadLine) -> Color {
    match road.class {
        RoadClass::Street if road.is_unpaved_street() => UNPAVED_ROAD_COLOR,
        RoadClass::Street => ROAD_COLOR,
        RoadClass::Alley if road.is_paved_path() => SIDEWALK_COLOR,
        RoadClass::Alley => ALLEY_COLOR,
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

/// [`RoadLine::lanes`] свободной функцией — её зовут как функция
/// (`lane_count(road)`) по всему `map/`: колея краски, границы полос, разрывы
/// ряда машин и островки спрашивают число полос у дороги. Сам подсчёт —
/// в модели: тег, а без него — дефолт по ширине ([`RoadLine::lanes`]).
pub fn lane_count(road: &RoadLine) -> u8 {
    road.lanes()
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
    let RoadStepsBuild {
        common,
        paint,
        steps,
    } = build_roads_steps(&skeleton, &[*verge]);
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
    spawn_road_common_layers(&mut commands, &mut meshes, &materials, common, paint);
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        (layers, report),
        verge.index,
        true,
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
        );
    }
}

/// Положить в мир **общие слои** дороги сплит-сборки
/// ([`build_roads_steps`]): один экземпляр на город, `RoadLayerTag` **без
/// `LodStep`** — ступень обочин их не прячет (`show_road_steps` читает
/// `LodStep`). Краска едет отдельно, со своими тегами ([`spawn_road_paint`]),
/// — дверь нужна отдельная именно из-за отсутствия метки у общих слоёв:
/// `spawn_road_meshes` всегда ставит `LodStep(step)`.
pub fn spawn_road_common_layers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    rest: Vec<LayerMesh>,
    paint: Vec<paint::PaintLayer>,
) {
    spawn_layers(
        commands,
        meshes,
        materials,
        rest.into_iter()
            .flat_map(|layer| layer.into_chunks(surface::MAP_CHUNK)),
        (RoadLayerTag, Visibility::Visible),
    );
    spawn_road_paint(commands, meshes, materials, paint);
}

/// Положить в мир слои краски: каждый **целиком** (`into_chunks`
/// полупрозрачную краску не режет) под `RoadLayerTag` со своим `PaintTag` —
/// тег прикреплён у рождения слоя (`paint::Painter::layers`) и приехал с
/// мешем. `LodStep` у краски нет: ступень обочин её не прячет, вид ведёт
/// `paint::show_paint`.
pub fn spawn_road_paint(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    paint: Vec<paint::PaintLayer>,
) {
    for layer in paint {
        spawn_layers(
            commands,
            meshes,
            materials,
            [layer.mesh],
            (RoadLayerTag, layer.tag),
        );
    }
}

/// Положить в мир ступенезависимые слои сборки: под `RoadLayerTag` с
/// `zoom::LodStep(step)`, плюс отчёт в лог. Дверь ступеней сплит-сборки
/// ([`spawn_map`], достройка [`show_road_steps`], пересборка
/// [`rebuild_roads`]) и слитой сборки витрин — форма
/// `buildings::spawn_building_meshes`: дверь нужна не по числу меток, а по
/// числу вызывающих. Общие слои спавнит [`spawn_road_common_layers`] —
/// без `LodStep`, ступень их не прячет; краску — [`spawn_road_paint`] со
/// своим `PaintTag`, у витрины тем же порядком, что и остальные слои.
///
/// `step` — ступень обочины, под которую собраны слои: обе ступени лежат в
/// мире сразу (собранные потоком загрузки, `map/prepare.rs`), скрытые, и
/// пересечение порога переключает видимость (`show_road_steps`), а не
/// пересобирает дорогу. `visible` — виден ли этот шаг при спавне.
pub fn spawn_road_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    (layers, report): (Vec<LayerMesh>, RoadReport),
    step: usize,
    visible: bool,
) {
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
        layers
            .into_iter()
            .flat_map(|layer| layer.into_chunks(surface::MAP_CHUNK)),
        (RoadLayerTag, LodStep(step), visibility),
    );
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
mod build;
mod common;
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
mod raw;
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
