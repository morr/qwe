//! Сборка дорожной области и её двери: вход [`build_roads`], скелет сети
//! ([`road_skeleton`], [`RoadSkeleton`]), сплит-дверь ступеней обочин
//! ([`build_roads_steps`], [`RoadStepsBuild`]), слитая дверь витрин и тестов
//! ([`build_roads_from`]) и побочные выходы сборки ([`RoadBuild`],
//! [`DrawnPieces`]) — их мир кладёт ресурсами.

use bevy::prelude::*;

use super::common::{CommonLayers, RoadCommon};
use super::drawn::{Axis, Drawn};
use super::gores::splitters::Splitter;
use super::junctions::Junctions;
use super::node_paint::CrossingMode;
use super::paint::Station;
use super::raw::mesh_raw_roads;
use super::ruts::RutLines;
use super::shape::RoadShape;
use super::tram_lay::TramTracks;
use super::turns::Turns;
use super::{
    RoadReport, RoadStyle, VergeZoomBucket, corners, gores, junctions, node_paint, paint, turns,
};
use crate::map::osm::MapData;
use crate::map::osm::parse::DrawBuildout;
use crate::map::parallel::in_parallel_owned;
use crate::map::surface::LayerMesh;

/// Всё, что выходит из одной сборки дорожной области ([`build_roads`]):
/// слои и отчёт — то же, что отдаёт [`mesh_roads`], — и три побочных
/// результата того же прохода, которые игра кладёт ресурсами.
#[derive(Default)]
pub struct RoadBuild {
    pub layers: Vec<LayerMesh>,
    /// Краска — вне `layers`, со своими тегами поведения у источника
    /// (`paint::Painter::layers`): мир кладёт её `spawn_road_paint`, витрина
    /// режет и кладёт так же. Полная картина с краской в z-порядке — у
    /// [`mesh_roads`], он приклеивает её обратно.
    pub paint: Vec<paint::PaintLayer>,
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
    if map.knobs.raw.disables(DrawBuildout::DecoratedRoads) {
        let (layers, report) = mesh_raw_roads(map, style);
        return RoadBuild {
            layers,
            paint: Vec::new(),
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
    pub(super) map: &'m MapData,
    pub(super) style: RoadStyle,
    pub(super) shape: RoadShape,
    /// Подготовленная сеть — самая дорогая часть скелета.
    pub(super) prepared: Drawn<'m>,
    /// Длина улицы у начала каждого way — по ней идут штрихи краски.
    pub(super) stations: Vec<Station>,
    /// Островки-крошки в треугольниках узлов (`corners::small_islands`).
    pub(super) islands: Vec<Vec<Vec2>>,
    /// Узлы и их краска, с островками подходов уже в разрывах: финал узлы
    /// только читает.
    pub(super) junctions: Junctions,
    /// Траектории манёвров — колея в узле.
    pub(super) turns: Turns,
    /// Островки на подходах к кольцам: одни и те же для узлов (здесь) и для
    /// клиньев у колец (финал).
    pub(super) splitters: Vec<Splitter>,
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
/// слитой сборки. Слитая дверь ([`build_roads_from`]) собирается из этого
/// выхода и остаётся бенчу, витринам и тестам — её строки и есть контроль
/// того, что сплит не тронул геометрию.
#[derive(Default)]
pub struct RoadStepsBuild {
    /// Слои, у ступеней бит-в-бит одинаковые (отпечатки записей 28 и 32):
    /// чисто общие — из своих сборщиков, смешанные (roads, sidewalks,
    /// alleys, unpaved_roads) — скругления первой ступени с дописанной
    /// один раз общей серединой.
    pub common: Vec<LayerMesh>,
    /// Краска общих слоёв — вне `common`, со своими тегами поведения у
    /// источника (`paint::Painter::layers`): мир кладёт её `spawn_road_paint`.
    pub paint: Vec<paint::PaintLayer>,
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
/// полный набор слоёв ступени в одном z-списке. Собирается **композицией
/// над сплит-дверью** ([`build_roads_steps`] на одной ступени) — выход тот
/// же, что до мирового сплита, бит-в-бит, и его строки (бенч
/// [`measure_roads`], витрины, тесты) остаются контролем того, что сплит
/// не тронул геометрию. Игра ходит сплит-дверью: общая середина
/// ([`RoadCommon`]) считается один раз на все ступени.
pub fn build_roads_from(skeleton: &RoadSkeleton<'_>, verge: VergeZoomBucket) -> RoadBuild {
    let RoadStepsBuild {
        common,
        paint,
        mut steps,
    } = build_roads_steps(skeleton, &[verge]);
    merged_picture(
        common,
        paint,
        steps
            .pop()
            .expect("слитая дверь собирает ровно одну ступень"),
    )
}

/// Слитая картина ступени из выхода сплит-двери: ступенезависимые слои
/// встают в общий z-список, хвост мощёных клиньев зелени возвращается из
/// слоя `sidewalk_scraps` в тротуары — формат слитой сборки (тот же z,
/// порядок кусков хвоста тот же). Отчёт ступени уже посчитан по полной
/// картине — свои слои плюс общие; краска и побочные выходы — при ней же.
fn merged_picture(
    mut common: Vec<LayerMesh>,
    paint: Vec<paint::PaintLayer>,
    step: StepRoadBuild,
) -> RoadBuild {
    let StepRoadBuild {
        mut layers,
        report,
        ruts,
        tram,
        pieces,
    } = step;
    // ── Склейка лоскутов ── у слитой сборки хвост лежит в тротуарах после
    // общей середины: слой ступени того же тротуарного материала на том же
    // z дописывается целиком, порядок треугольников сохраняется
    let scraps_at = layers
        .iter()
        .position(|layer| layer.name == "sidewalk_scraps")
        .expect("ступень несёт свой слой мощёных клиньев");
    let sidewalks_at = common
        .iter()
        .position(|layer| layer.name == "sidewalks")
        .expect("общие слои несут тротуары");
    let scraps = layers.swap_remove(scraps_at);
    common[sidewalks_at].builder.append(&scraps.builder);
    common.append(&mut layers);
    common.sort_by(|a, b| a.z.total_cmp(&b.z));
    RoadBuild {
        layers: common,
        paint,
        report,
        ruts,
        tram,
        pieces,
    }
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
        paint,
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
        paint,
        steps,
    }
}
