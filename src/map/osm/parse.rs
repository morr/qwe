//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов. Деревья по разобранной карте сажает
//! `super::planting`.

use std::collections::{HashMap, HashSet};

use bevy::math::Vec2;
use bevy::prelude::{Reflect, ReflectDefault, ReflectResource, Resource};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use super::contours::OsmContours;
use super::planting::plant_trees;
use crate::city::City;
use crate::grid::DEFAULT_NAVTILE_SIZE;
use crate::map::cars::district::Districts;
use crate::map::osm::entrances::generate_entrances;
use crate::map::osm::model::{
    AreaKind, FenceLine, MapData, PipeLine, PolyArea, RailLine, RoadArea, RoadClass, RoadLine,
    RoadNode, Structure, TrafficSide, TreeCompose, TreeNode, TreeRow, TreeRowLayout, WallLine,
    WaterLine, point_in_area, point_in_polygon, ring_bounds, ring_vertex_mean,
};
use crate::map::osm::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};
use crate::map::roads::network::sections::{self, SectionReport};
use crate::map::roads::rings;
use crate::map::roads::shape::LANE_WIDTH_DEFAULT;

/// Ширина стены Кремля, м.
const WALL_WIDTH: f32 = 3.0;
/// Совпадение концов way при сборке колец, м (общие OSM-узлы дают
/// идентичные координаты; эпсилон страхует от шума проекции).
const RING_JOIN_EPSILON: f32 = 0.01;

/// Шаг квантования координат при привязке входа к зданию, 1/м. Вход в OSM —
/// как правило общий узел контура здания (Тула 82%, Берлин 79%, Париж 65%),
/// так что после одной и той же проекции координаты совпадают точно;
/// сантиметровая сетка — страховка от шума f32, а не поиск ближайшего.
const ENTRANCE_SNAP_SCALE: f32 = 100.0;

/// Разбор — две половины, и между ними шов.
///
/// Первая читает элементы Overpass в сырую [`MapData`] ([`read_elements`]),
/// вторая гоняет по ней доводочные проходы в их единственно верном порядке
/// ([`finish_parse`]). До шва обе лежали одним телом на сто двадцать пять
/// строк, из которых сорок пять были телеметрией, и позвать проход в одиночку
/// было не то чтобы нельзя — просто не за что было взяться: у стадии не было
/// имени. Все шестьдесят тестов разбора поэтому гоняли конвейер целиком и
/// адресовали дома по их месту в фикстуре.
pub fn parse(json: &str, city: City, knobs: ParseKnobs) -> Result<MapData, String> {
    let response: OverpassResponse =
        serde_json::from_str(json).map_err(|error| format!("overpass json: {error}"))?;
    Ok(parse_response(&response, city, knobs))
}

/// Входы разбора, которых нет в ответе Overpass. Раньше обе были процессными
/// глобалями, которые разбор читал сам (`shape::lane_width()`,
/// `grid::navtile_size()`): у `parse(json, city)` было два невидимых входа, и
/// тест не мог назвать ширину полосы, не заперев глобаль мьютексом —
/// `cargo test` многопоточный. Теперь их называет вызывающий: поток загрузки —
/// осевшими ручками (`loading::start_job`), витрина дорог — своей ширины
/// полосы, всё прочее — [`Default`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParseKnobs {
    /// Ширина полосы улицы, м — ручка `Lane width` (`RoadShape::lane_width`).
    /// Её читают сечения (шаг 0 [`finish_parse`]), а по ширине дорог
    /// двигаются дома, кварталы и стоянки.
    pub lane_width: f32,
    /// Размер навтайла, м (`NavtileBase::size`): зазор, который генератор
    /// дверей проверяет перед стеной (шаг 7).
    pub navtile: f32,
    /// Режим **сырого OSM** ([`RawOsm`]): какие наши достройки выключены.
    pub raw: RawOsm,
}

impl ParseKnobs {
    /// Входы по умолчанию — дефолты ручек (3.3 м, 2 м, сырой OSM выключен).
    /// Константой, а не только [`Default`]: тест называет её там, где нужна
    /// константа.
    pub const DEFAULT: Self = Self {
        lane_width: LANE_WIDTH_DEFAULT,
        navtile: DEFAULT_NAVTILE_SIZE,
        raw: RawOsm::Off,
    };
}

/// **Сырой OSM** — карта без наших достроек и правок данных, чтобы на кривом
/// месте ответить «виноваты данные или наш разбор» за одно нажатие (строка
/// `Raw OSM` секции World build вкладки Debug; витрина — `ROADS_RAW=parse|draw`).
/// Меняет вход разбора, поэтому переключение перезагружает мир, как навтайл.
///
/// Два уровня:
/// - [`RawOsm::Parse`] — [`finish_parse`] пропускает все доводочные проходы:
///   выпрямление хвостов колец, вывод сечений (ширину по полосам получает только
///   участок с тегом `lanes`), утопленников, достроенные тротуары и покрытие
///   дорожек (тротуар остаётся только по тегу `sidewalk*`), веру храмов,
///   выпрямление домиков, сдвиги домов, кварталов и стоянок, засев карманов,
///   генерацию дверей. Размеченные двери (`attach_entrances`) остаются — это
///   данные;
/// - [`RawOsm::Draw`] — то же, и ещё выключены достройки отрисовки: дорога —
///   простая лента по оси OSM без скруглений, склейки узлов, пар половин,
///   разделительных, устьев, газонов обочин и разметки (`roads::mesh_raw_roads`);
///   стоянка и площадка — своим полигоном без раскладки мест и разметки поля;
///   ни машин, ни стоящих вагонов (`wagons::mesh_wagons`), ни сгенерированных
///   деревьев (лес и аллеи не сажаются, остаются только деревья-ноды OSM).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "raw_osm")]
pub enum RawOsm {
    #[default]
    Off,
    Parse,
    Draw,
}

impl RawOsm {
    pub const ALL: [Self; 3] = [Self::Off, Self::Parse, Self::Draw];

    /// Пропускает ли разбор доводочные проходы — оба уровня.
    pub fn skips_passes(self) -> bool {
        self != Self::Off
    }

    /// Выключены ли достройки отрисовки — только второй уровень.
    pub fn draws_raw(self) -> bool {
        self == Self::Draw
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Parse,
            Self::Parse => Self::Draw,
            Self::Draw => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Parse => "parse",
            Self::Draw => "parse+draw",
        }
    }

    /// Уровень по слову (`off`, `parse`, `draw`) — для переменной окружения
    /// витрины.
    pub fn from_word(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "" => Some(Self::Off),
            "parse" | "1" => Some(Self::Parse),
            "draw" | "2" => Some(Self::Draw),
            _ => None,
        }
    }
}

impl Default for ParseKnobs {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// [`parse`] уже десериализованного ответа. Отдельной дверью — ради витрины
/// дорог: она читает выгрузку города один раз и режет из неё окна
/// ([`super::crop`]), и гонять каждое окно обратно через JSON было бы
/// круговой поездкой ни за чем.
pub fn parse_response(response: &OverpassResponse, city: City, knobs: ParseKnobs) -> MapData {
    let bounds = GeoBounds::for_city(city);

    let (mut map, pending, read) = read_elements(response, &bounds);
    eprint!("{read}");
    let passes = finish_parse(&mut map, &pending, knobs);
    eprint!("{passes}");
    map
}

/// Что сказал элементный цикл — значением, а не двумя `eprintln!`.
struct ReadReport {
    /// `None` — зеркало не отдало `is_in`; карта рисуется правосторонней.
    traffic_side: Option<TrafficSide>,
    unclosed_rings: usize,
    /// Вершин в снимке контуров OSM ([`OsmContours`]) и цена копии.
    contour_points: usize,
    contouring: std::time::Duration,
}

impl std::fmt::Display for ReadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // разбор по полям, а не `self.…`: подстановка тогда читается по имени,
        // а не счётом позиций — как у всех прочих отчётов `map/*`
        let Self {
            traffic_side,
            unclosed_rings,
            contour_points,
            contouring,
        } = self;
        writeln!(
            f,
            "osm parse: {contour_points} contour points kept for the OSM overlay in {contouring:?}"
        )?;
        if traffic_side.is_none() {
            // не error: зеркало без областей отдаёт пустой `is_in`, а карта без
            // стороны движения всё равно рисуется
            writeln!(
                f,
                "osm parse: no driving_side in the answer, assuming right-hand traffic"
            )?;
        }
        if *unclosed_rings > 0 {
            // не error: кольца, порванные краем bbox, ожидаемы
            writeln!(
                f,
                "osm parse: {unclosed_rings} unclosed relation rings skipped"
            )?;
        }
        Ok(())
    }
}

/// Что элементный цикл прочёл, но положить в `MapData` ещё не может.
#[derive(Default)]
struct Pending {
    /// Входы: Overpass отдаёт ноды раньше way, так что на момент разбора ноды
    /// зданий ещё нет.
    entrances: Vec<Vec2>,
}

/// Элементы Overpass — в сырую `MapData` и то, что ещё некуда положить
/// ([`Pending`]).
///
/// **Ничего не доводит.** Дома ещё стоят в воде, храмы без веры, контуры
/// косые, дверей нет, деревья не посажены — всё это [`finish_parse`].
fn read_elements(
    response: &OverpassResponse,
    bounds: &GeoBounds,
) -> (MapData, Pending, ReadReport) {
    let mut map = MapData::default();
    let traffic_side = driving_side(&response.elements);
    if let Some(side) = traffic_side {
        map.traffic_side = side;
    }
    let mut unclosed_rings = 0usize;
    let mut pending = Pending::default();

    for element in &response.elements {
        match element.kind.as_str() {
            "node" => {
                if let Some(position) = parse_entrance(element, bounds) {
                    pending.entrances.push(position);
                }
                if let Some(node) = parse_tree_node(element, bounds) {
                    map.tree_nodes.push(node);
                }
                if let Some(structure) = parse_structure_node(element, bounds) {
                    map.structures.push(structure);
                }
                if let Some(node) = parse_road_node(element, bounds) {
                    map.road_nodes.push(node);
                }
            }
            "way" => parse_way(element, bounds, &mut map),
            "relation" => parse_relation(element, bounds, &mut map, &mut unclosed_rings),
            _ => {}
        }
    }

    // снимок для оверлея контуров — до единого доводочного прохода
    let started = std::time::Instant::now();
    map.osm_contours = OsmContours::of(&map);
    let report = ReadReport {
        traffic_side,
        unclosed_rings,
        contour_points: map.osm_contours.points(),
        contouring: started.elapsed(),
    };
    (map, pending, report)
}

/// Что дала посадка — то, чем была самая длинная строка лога.
struct PlantedReport {
    woods: usize,
    standalone: usize,
    tree_nodes: usize,
    /// По одной на политику размещения аллей, в порядке [`TreeRowLayout::ALL`]
    /// — массивом той же длины, а не `Vec`: «столько же и в том же порядке»
    /// держит тип, а не эта строчка.
    rows: [usize; TreeRowLayout::ALL.len()],
    tree_rows: usize,
    asked: usize,
}

/// Что сделали доводочные проходы — значением, а не восемью `eprintln!`.
///
/// Те же счётчики, что уходили в лог, но теперь их можно сравнить в тесте: до
/// этого единственным способом узнать, сколько домов отодвинулось от
/// тротуаров, было прочесть строку на stderr.
struct PassReport {
    /// Режим сырого OSM, с которым шёл разбор: не `Off` — счётчики пропущенных
    /// проходов нулевые не потому, что делать было нечего.
    raw: RawOsm,
    /// Y-подходов с хвостом, выпрямленных в обычный «Y»
    /// (`roads::rings::straighten_tails`).
    straightened: rings::Straightened,
    sections: SectionReport,
    drowned: usize,
    /// Пар путей, сшитых через дорогу (`rails::stitch_rail_gaps`).
    stitched_rails: usize,
    sidewalks: InferredSidewalks,
    pavements: pavements::InferredPavements,
    separate: verges::SeparateSidewalks,
    faiths_guessed: usize,
    entrances_found: usize,
    entrances_orphaned: usize,
    squared: usize,
    squaring: std::time::Duration,
    pulled: PulledHouses,
    pulling: std::time::Duration,
    stretched: StretchedAreas,
    stretching: std::time::Duration,
    /// Замощённые стоянки (`lots::pave_lots`) — своё поле, а не часть
    /// [`StretchedAreas`]: у прохода свой вызов в шаге 6 и свой договор с
    /// картой.
    lots: lots::PavedLots,
    paving: std::time::Duration,
    /// Карманов земли у дорог, засеянных травой
    /// (`pockets::fill_ground_pockets`).
    sown: usize,
    sowing: std::time::Duration,
    generated: usize,
    generating: std::time::Duration,
    planted: PlantedReport,
    planting: std::time::Duration,
}

impl std::fmt::Display for PassReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // разбор по полям, а не `self.…`: в строке посадки шесть подстановок, и
        // по именам они читаются, а по позициям — только счётом
        let Self {
            raw,
            straightened,
            sections,
            drowned,
            stitched_rails,
            sidewalks,
            pavements,
            separate,
            faiths_guessed,
            entrances_found,
            entrances_orphaned,
            squared,
            squaring,
            pulled,
            pulling,
            stretched,
            stretching,
            lots,
            paving,
            sown,
            sowing,
            generated,
            generating,
            planted,
            planting,
        } = self;
        if raw.skips_passes() {
            writeln!(
                f,
                "osm parse: RAW OSM ({}) — finishing passes skipped, only tagged sidewalks and mapped doors",
                raw.label()
            )?;
        }
        let rings::Straightened { tails, elapsed } = straightened;
        writeln!(
            f,
            "osm parse: {tails} roundabout Y approaches straightened off their tails in {elapsed:?}"
        )?;
        writeln!(f, "{sections}")?;
        if *drowned > 0 {
            writeln!(
                f,
                "osm parse: {drowned} buildings dropped as standing entirely in water"
            )?;
        }
        if *stitched_rails > 0 {
            writeln!(
                f,
                "osm parse: {stitched_rails} rail tracks broken at a street stitched across it"
            )?;
        }
        writeln!(f, "{sidewalks}")?;
        writeln!(f, "{pavements}")?;
        writeln!(f, "{separate}")?;
        if *faiths_guessed > 0 {
            writeln!(
                f,
                "osm parse: {faiths_guessed} places of worship took their faith from the city"
            )?;
        }
        if *entrances_orphaned > 0 {
            // ожидаемо: вход бывает отдельной нодой у крыльца, а не узлом
            // контура, либо принадлежит зданию, которое не попало в bbox
            writeln!(
                f,
                "osm parse: {entrances_orphaned} of {entrances_found} entrances match no building"
            )?;
        }
        if *squared > 0 {
            writeln!(
                f,
                "osm parse: {squared} skewed small houses squared into rectangles and L shapes in {squaring:?}"
            )?;
        }
        let PulledHouses {
            moved,
            partly,
            left,
        } = pulled;
        writeln!(
            f,
            "osm parse: {moved} buildings pulled off the sidewalks ({partly} of them only part of the way), {left} left standing on them, in {pulling:?}"
        )?;
        let StretchedAreas {
            blocks,
            cut,
            cutting,
        } = stretched;
        writeln!(
            f,
            "osm parse: {blocks} block vertices pulled to the drawn road edge, {cut} blocks cut off the verges, in {stretching:?} (the verge cut alone: {cutting:?})"
        )?;
        let lots::PavedLots { grown, trimmed } = lots;
        writeln!(
            f,
            "osm parse: {grown} parking lots paved up to their roads, {trimmed} only stepped back from the houses on them, in {paving:?}"
        )?;
        writeln!(
            f,
            "osm parse: {sown} ground pockets by the roads sown with grass in {sowing:?}"
        )?;
        let attached = entrances_found - entrances_orphaned;
        writeln!(
            f,
            "osm parse: {attached} entrances attached, {generated} generated in {generating:?}"
        )?;
        // «посажено меньше, чем запрошено» — лес упёрся в насыщение, потолок
        // плотности стоит выше достижимого (см. `planting::TREE_MIN_SPACING`).
        // Аллеи считаются отдельно и под обе политики: `kept` = `slid`
        // означает, что сдвигать было нечего, а `0 in K tree rows` при `K > 0`
        // — что тег доехал, а посадка по нему не встала никуда. Одиночные ноды
        // выбывают штатно — в лесу и у аллей дерево уже посажено процедурно
        let PlantedReport {
            woods,
            standalone,
            tree_nodes,
            rows,
            tree_rows,
            asked,
        } = planted;
        let counts = rows
            .iter()
            .map(usize::to_string)
            .collect::<Vec<String>>()
            .join("/");
        writeln!(
            f,
            "osm parse: {woods} trees planted of {asked} asked, {standalone} standalone of \
             {tree_nodes} tree nodes, {counts} in {tree_rows} tree rows \
             (keep/slide x osm/slider) in {planting:?}"
        )
    }
}

/// Доводочные проходы по сырой карте — девять, плюс сборка деревьев в конце, —
/// **и этот порядок и есть их интерфейс**. До этой функции он жил заметками в
/// трёх doc-комментариях из восьми и не был записан целиком нигде.
///
/// Почему именно так, сверху вниз:
///
/// 0. **Улицы и сечения** (`map::roads::network::sections`) — самыми первыми:
///    ширина дороги выводится из числа полос, а её читают шаги 5 и 6 (тротуар
///    отодвигает дома, край дороги притягивает кварталы и стоянки). Сеть улиц,
///    собранная здесь, читается и тротуаром, отданным отдельной дорожке
///    (шаг 1), — по ней проба знает, чей след для неё чужой. Домов этот
///    шаг не касается, так что ставить его раньше утопленников ничему не мешает.
///    Ещё раньше, прямо перед ним, **выпрямляются Y-подходы к кольцам**
///    (`map::roads::rings::straighten_tails`): проход двигает узлы и режет way,
///    а сеть улиц собирается по готовым way, так что после сечений он бы её
///    сломал.
/// 1. **Утопленники** уходят первыми: дом, целиком стоящий в воде, не должен
///    получить ни веры, ни двери, ни выпрямленного контура — всё это работа
///    по дому, которого не будет.
///    **Пути, оборванные на дороге, сшиваются** сразу за ними
///    ([`rails::stitch_rail_gaps`]): проход читает только осевые улиц и
///    путей, а место — любое после сечений (ему нужен класс проезжей части);
///    дальше пути в разборе никто не двигает.
///    **Тротуары без тега** решаются сразу за ними ([`infer_sidewalks`]): мера
///    — этажность застройки вокруг, и утонувший дом в неё входить не должен;
///    а читают решение шаги 5 и 6 (дом отъезжает только от нарисованного
///    тротуара, квартал дотягивается под него же).
///    **Покрытие дорожек без тега** ([`pavements::infer_pavements`]) — там же, хотя
///    место ему любое: зелень, которую он спрашивает, дальше не двигается, а
///    читает решение только рендер.
///    **Тротуар, отданный отдельной дорожке** ([`verges::measure_footways_beside_streets`]),
///    — сразу за покрытием: он спрашивает, мощёная ли дорожка вдоль кромки, а
///    читают его, как и тротуар по застройке, шаги 5 и 6.
/// 2. **Вера** — до дверей и до выпрямления: она собирает храм из частей
///    (`faiths::resolve_faiths` зовёт `absorb_annexes` внутри себя), а часть,
///    ставшая приделом, дальше читается иначе.
/// 3. **Разметанные двери** прикладываются к контурам, пока те ещё сырые: у
///    входа координата **ноды**, а не вершины, и ищется он по тому же
///    сантиметровому ключу.
/// 4. **Выпрямление косых домиков** — после разметки дверей (уже
///    приложенную дверь перенос контура уносит с собой по тому же ключу; точное
///    `==` молча оставило бы её на месте) и до генерации дверей и посадки
///    деревьев (тем нужен уже выпрямленный контур).
/// 5. **Отодвигание домов от тротуаров** — после выпрямления (косой дом
///    сначала становится прямым, потом отъезжает) и до всего, что читает
///    контур.
/// 6. **Подтягивание кварталов и стоянок к дорогам** — где угодно в хвосте: ни
///    `landuse`, ни `parking` не трогают ни навмеш, ни двери, ни посадку.
///    Стоит здесь, потому что дома к этому моменту уже на своих местах, а
///    контуры стоянок — **после** выпрямления домиков (шаг 4): `vertex_uses`
///    считает общие вершины по сырым контурам стоянок, и дотянутый край не
///    должен менять то, какие дома выпрямились. Шаг — две строки: кварталы
///    (`pull_areas_to_roads`), а сразу за ними стоянки (`lots::pave_lots`) —
///    другим контекстом: узкий проход получает [`lots::PaveLotsCtx`] со
///    ссылками на то, что читает, а не `&mut MapData` целиком.
/// 7. **Генерация дверей** — по уже окончательным контурам: дверь ставится по
///    стене того дома, который останется на карте.
/// 8. **Посадка деревьев** — тоже по окончательным контурам: дерево обходит
///    дом там, где дом стоит после выпрямления и сдвига.
/// 9. **Сборка деревьев по составу по умолчанию** (`compose_trees`) — только
///    после посадки и после того, как её три набора легли в `map`: парсер о
///    панелях ничего не знает, но и отдавать `MapData` с пустым `trees` не
///    должен — иначе каждый читатель обязан помнить про отдельный шаг сборки.
///    Выбранный игроком состав доложит `map::trees::recompose_row_trees`, и
///    только если он другой.
///
/// **`vertex_uses` считается дважды, и это не расточительство.** Шаги 4 и 5
/// оба спрашивают «эта вершина общая?», и между ними контуры **двигаются**:
/// выпрямленный дом уносит свои вершины на новые места, и счёт, снятый до
/// него, отвечал бы про старую карту.
///
/// Входы вне ответа Overpass — только `knobs`: ширину полосы берёт шаг 0,
/// навтайл — шаг 7. Глобалей проходы не читают.
///
/// **Сырой OSM** (`knobs.raw`, [`RawOsm`]) пропускает все шаги, кроме
/// размеченных дверей (шаг 3) и посадки (шаг 8); сеть улиц склеивается без
/// вывода сечений (`sections::apply_raw`), а тротуар без тега снимается
/// ([`drop_untagged_sidewalks`]). На втором уровне посадка оставляет только
/// деревья-ноды OSM.
fn finish_parse(map: &mut MapData, pending: &Pending, knobs: ParseKnobs) -> PassReport {
    map.knobs = knobs;
    let entrances = &pending.entrances;
    // сырой OSM (`RawOsm`): доводочные проходы пропускаются все, кроме
    // размеченных дверей — это данные, — и сборки сети улиц, которая ничего в
    // данных не меняет. Пропущенный проход отчитывается нулями
    let raw = knobs.raw.skips_passes();
    let straightened = if raw {
        Default::default()
    } else {
        rings::straighten_tails(&mut map.roads)
    };
    let sections = if raw {
        sections::apply_raw(map, knobs.lane_width)
    } else {
        sections::apply(map, knobs.lane_width)
    };
    let drowned = if raw { 0 } else { drop_buildings_in_water(map) };
    // переезд, которого нет в OSM: путь, оборванный на дороге, сшивается
    let stitched_rails = if raw { 0 } else { rails::stitch_rail_gaps(map) };
    let (sidewalks, pavements, separate, faiths_guessed) = if raw {
        // тротуар — только по тегу: достроенный без тега снимается
        let dropped = drop_untagged_sidewalks(&mut map.roads);
        (
            InferredSidewalks {
                asked: dropped,
                dropped,
            },
            Default::default(),
            Default::default(),
            0,
        )
    } else {
        // мера квартала — явный вход прохода: один `Districts` на все его
        // вопросы «сколько вокруг этажей», построенный по домам, оставшимся
        // после утопленников (утонувший дом в неё входить не должен)
        let sidewalks = {
            let districts = Districts::new(&map.buildings);
            infer_sidewalks(&mut map.roads, &districts)
        };
        let pavements = pavements::infer_pavements(pavements::InferPavementsCtx::of(map));
        let separate = verges::measure_footways_beside_streets(&mut map.roads, &map.network);
        let faiths_guessed = faiths::resolve_faiths(faiths::ResolveFaithsCtx::of(map));
        (sidewalks, pavements, separate, faiths_guessed)
    };
    let entrances_orphaned = attach_entrances(map, entrances);

    let started = std::time::Instant::now();
    let squared = if raw { 0 } else { square_skewed_houses(map) };
    let squaring = started.elapsed();

    let started = std::time::Instant::now();
    let pulled = if raw {
        PulledHouses::default()
    } else {
        pull_houses_off_sidewalks(map)
    };
    let pulling = started.elapsed();

    let started = std::time::Instant::now();
    let stretched = if raw {
        StretchedAreas::default()
    } else {
        pull_areas_to_roads(map)
    };
    let stretching = started.elapsed();

    // стоянки — второй строкой того же шага 6, сразу за кварталами и их
    // обочинами: дотянутые кварталы уже покрывают свою землю. Данные —
    // контекстом (`PaveLotsCtx`), а не картой целиком: узкий проход получает
    // ссылку на то, что читает
    let started = std::time::Instant::now();
    let lots = if raw {
        lots::PavedLots::default()
    } else {
        lots::pave_lots(lots::PaveLotsCtx::of(map))
    };
    let paving = started.elapsed();

    // после кварталов и стоянок: дотянутые, они уже покрывают свою землю
    let started = std::time::Instant::now();
    let sown = if raw {
        0
    } else {
        pockets::fill_ground_pockets(map)
    };
    let sowing = started.elapsed();

    // размеченных дверей в OSM единицы процентов — остальным дом получает свои
    // по замеру когорт, см. `entrances/`
    let started = std::time::Instant::now();
    let generated = if raw {
        0
    } else {
        generate_entrances(map, knobs.navtile)
    };
    let generating = started.elapsed();

    let started = std::time::Instant::now();
    let (standalone, mut woods, mut rows, asked) = plant_trees(map);
    // второй уровень: сгенерированных деревьев нет — лес и аллеи не
    // сажаются, остаются деревья-ноды OSM
    if knobs.raw.draws_raw() {
        woods.clear();
        rows = Default::default();
    }
    let planting = started.elapsed();
    let planted = PlantedReport {
        woods: woods.len(),
        standalone: standalone.len(),
        tree_nodes: map.tree_nodes.len(),
        rows: TreeRowLayout::ALL.map(|layout| rows.get(layout).len()),
        tree_rows: map.tree_rows.len(),
        asked,
    };
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;
    // шаг 9, довод — в списке над функцией
    map.compose_trees(TreeCompose::default());

    PassReport {
        raw: knobs.raw,
        straightened,
        sections,
        drowned,
        stitched_rails,
        sidewalks,
        pavements,
        separate,
        faiths_guessed,
        entrances_found: entrances.len(),
        entrances_orphaned,
        squared,
        squaring,
        pulled,
        pulling,
        stretched,
        stretching,
        lots,
        paving,
        sown,
        sowing,
        generated,
        generating,
        planted,
        planting,
    }
}

/// Дома, целиком стоящие в воде, выбрасываются. В OSM это плавучие рестораны и
/// дебаркадеры (`HMS Belfast` в Лондоне, `Café Barge` в Париже), а в Туле —
/// одинокий сарай посреди Верхнего пруда. Навмеш заливает воду непроходимой,
/// так что до дверей такого дома пешка всё равно не дойдёт, а коробка посреди
/// пруда читается как баг рендера.
///
/// Критерий — **все** вершины контура в воде: дом, зацепившийся за берег
/// (пирс, набережная, дом на сваях у кромки), остаётся. Выброшенных единицы:
/// Тула 1, Берлин 6, Нью-Йорк 17, Лондон и Париж по 28, Токио 0.
///
/// Место в конвейере — **шаг 1** [`finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
fn drop_buildings_in_water(map: &mut MapData) -> usize {
    // AABB-прекомпьют: воды десятки полигонов, зданий десятки тысяч, и почти
    // каждое отсеивается на первой же вершине, не доходя до point-in-polygon
    let bounds: Vec<(Vec2, Vec2)> = map
        .water
        .iter()
        .map(|area| ring_bounds(&area.outer))
        .collect();

    let MapData {
        buildings, water, ..
    } = map;
    let before = buildings.len();
    buildings.retain(|building| {
        !building.outer.iter().all(|point| {
            water.iter().zip(&bounds).any(|(area, &(min, max))| {
                point.x >= min.x
                    && point.x <= max.x
                    && point.y >= min.y
                    && point.y <= max.y
                    && point_in_area(*point, area)
            })
        })
    });
    before - buildings.len()
}

/// Нода `entrance=*` → позиция на карте. Не вход или значение из
/// [`NON_WALKABLE_ENTRANCES`] — `None`.
fn parse_entrance(element: &Element, bounds: &GeoBounds) -> Option<Vec2> {
    let entrance = element.tags.get("entrance")?.as_str();
    if NON_WALKABLE_ENTRANCES.contains(&entrance) {
        return None;
    }
    Some(bounds.project(element.lat?, element.lon?))
}

/// Нода `man_made=*` → цилиндр промзоны. Контура у ноды нет, поэтому радиус
/// берётся из тега `diameter`, а если и его нет — типовой для рода
/// ([`structure_size`]). В Туле нодами размечены четыре заводские трубы из
/// восьми.
fn parse_structure_node(element: &Element, bounds: &GeoBounds) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let (radius, height) = structure_size(kind);
    Some(Structure {
        at: bounds.project(element.lat?, element.lon?),
        radius: structure_radius(&element.tags).unwrap_or(radius),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
        osm_id: Some(element.id),
    })
}

/// Way `man_made=*` → тот же цилиндр, но радиус считается по контуру: он
/// заведомо честнее тега, которого в данных обычно и нет.
///
/// Центр — [`ring_vertex_mean`], а не центроид площади
/// ([`ring_area_centroid`]): контур цилиндра в OSM рисуют равномерным
/// многоугольником, на нём это одно и то же, а у вытянутого контура (силосный
/// корпус, размеченный прямоугольником) среднее вершин ближе к тому, что глаз
/// считает серединой.
fn parse_structure_way(element: &Element, points: &[Vec2]) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let ring = as_ring(points)?;
    let at = ring_vertex_mean(&ring)?;
    let spread = ring.iter().map(|point| at.distance(*point)).sum::<f32>() / ring.len() as f32;
    let (_, height) = structure_size(kind);
    Some(Structure {
        at,
        radius: spread.max(f32::EPSILON),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
        osm_id: Some(element.id),
    })
}

/// Нода `highway=crossing|traffic_signals|…` → дорожный узел ([`RoadNode`]);
/// вид — по белому списку `road_node_kind`.
fn parse_road_node(element: &Element, bounds: &GeoBounds) -> Option<RoadNode> {
    Some(RoadNode {
        kind: road_node_kind(&element.tags)?,
        pos: bounds.project(element.lat?, element.lon?),
        osm_id: Some(element.id),
    })
}

/// Нода `natural=tree` → одиночное дерево. Сажает его (и отсеивает
/// продублированные процедурной посадкой) `planting::plant_standalone`.
fn parse_tree_node(element: &Element, bounds: &GeoBounds) -> Option<TreeNode> {
    if element.tags.get("natural").map(String::as_str) != Some("tree") {
        return None;
    }
    Some(TreeNode {
        pos: bounds.project(element.lat?, element.lon?),
        radius: crown_radius(&element.tags),
    })
}

/// Раскладка входов по зданиям: вход ищется среди вершин контуров, потому что
/// в OSM он и есть узел контура. Возвращает число входов, не нашедших дом.
///
/// Общий узел двух домов (сплошная застройка) попадает в таблицу один раз —
/// вход достанется одному из них, и это не важно: дверь всё равно там же.
///
/// Совпадающие входы схлопываются: в Париже встречаются две ноды `entrance` в
/// одной точке (замер по выгрузке — минимальный зазор 0.00 м), а две двери на
/// одном месте — это две одинаковых цели для пешек и лишний кружок в оверлее.
fn attach_entrances(map: &mut MapData, entrances: &[Vec2]) -> usize {
    let mut by_vertex: HashMap<(i32, i32), usize> = HashMap::new();
    for (index, building) in map.buildings.iter().enumerate() {
        for &vertex in &building.outer {
            by_vertex.insert(vertex_key(vertex), index);
        }
    }

    let mut orphaned = 0;
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    for &entrance in entrances {
        let Some(&index) = by_vertex.get(&vertex_key(entrance)) else {
            orphaned += 1;
            continue;
        };
        // дубль считаем привязанным, а не сиротой: дом он нашёл
        if taken.insert(vertex_key(entrance)) {
            map.buildings[index].entrances.push(entrance);
        }
    }
    orphaned
}

/// Ключ вершины на сантиметровой сетке: общий OSM-узел двух контуров, линий или
/// входа после одной проекции даёт один и тот же ключ. Одна привязка на всех —
/// [`attach_entrances`] ищет дом этим же ключом (см. [`ENTRANCE_SNAP_SCALE`]).
fn vertex_key(point: Vec2) -> (i32, i32) {
    (
        (point.x * ENTRANCE_SNAP_SCALE).round() as i32,
        (point.y * ENTRANCE_SNAP_SCALE).round() as i32,
    )
}

/// Сколько контуров и линий карты проходит через каждую вершину: `> 1` —
/// вершина общая (сплошная застройка, забор по стене, арка, тропа до угла).
///
/// Считаются здания, все линейные слои и **три площадных из восьми** —
/// [`MapData::parking`], [`MapData::pitches`], [`MapData::water`]. Перечень
/// явный, а не «все контуры карты», потому что решает тут не модель, а кадр:
/// край этих трёх нарисован собственной поверхностью с разметкой, а на стоянке
/// ещё и машинами (`cars::fill_lots`), так что дом, отъехавший от него, виден.
/// `parks`, `grass`, `woods`, `sand` не считаются — дом лежит поверх заливки,
/// и шва под ним не видно; `landuse` не считается по решению коммита 174ab8a
/// (частные дома сплошь и рядом обведены по границе квартала), и правило выше
/// его расширяет, а не спорит с ним.
fn vertex_uses(map: &MapData) -> HashMap<(i32, i32), u32> {
    let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
    let mut count = |points: &[Vec2]| {
        let unique: HashSet<(i32, i32)> = points.iter().map(|point| vertex_key(*point)).collect();
        for vertex in unique {
            *uses.entry(vertex).or_default() += 1;
        }
    };
    for building in &map.buildings {
        count(&building.outer);
        building.holes.iter().for_each(|hole| count(hole));
    }
    map.roads.iter().for_each(|line| count(&line.points));
    map.rails.iter().for_each(|line| count(&line.points));
    map.walls.iter().for_each(|line| count(&line.points));
    map.fences.iter().for_each(|line| count(&line.points));
    map.pipes.iter().for_each(|line| count(&line.points));
    map.water_lines.iter().for_each(|line| count(&line.points));
    for layer in [&map.parking, &map.pitches, &map.water] {
        for area in layer {
            count(&area.outer);
            area.holes.iter().for_each(|hole| count(hole));
        }
    }
    uses
}

/// Ячейка сетки отрезков улиц, м.
const SIDEWALK_CELL: f32 = 32.0;

/// Звено линии карты и то расстояние от его оси, дальше которого оно не
/// достаёт. Один тип на все проходы этого дерева, потому что вся геометрия у
/// них одна — «отрезок плюс радиус вокруг него», — а вот **чем** измерена
/// `reach`, решает тот, кто звено сложил: полоса улицы с тротуаром и зазором
/// ([`pull_houses_off_sidewalks`]), внешний край нарисованного полотна
/// ([`pull_areas_to_roads`]), радиус запрета ([`house_pull::Obstacles`]).
#[derive(Clone, Copy)]
struct Link {
    from: Vec2,
    to: Vec2,
    reach: f32,
}

/// На сколько метров дотянутый край площади — квартала или стоянки —
/// заводится **под** полотно, м.
/// Лента рисуется по сглаженной оси (`roads::centerline`), а зазор меряется по
/// сырым точкам OSM — без запаса на повороте осталась бы щель в сантиметр.
const LANDUSE_OVERLAP: f32 = 0.5;

fn project_points(points: &[LatLon], bounds: &GeoBounds) -> Vec<Vec2> {
    points
        .iter()
        .map(|point| bounds.project(point.lat, point.lon))
        .collect()
}

/// Закрытая полилиния way → открытое кольцо (без повторённой последней точки).
fn as_ring(points: &[Vec2]) -> Option<Vec<Vec2>> {
    if points.len() < 4 || points.first() != points.last() {
        return None;
    }
    Some(points[..points.len() - 1].to_vec())
}

/// Повторы и возвраты точек кольца ближе этого — игла, м.
const RING_SLIVER: f32 = 0.05;

/// Кольцо без вырожденных игл: подряд идущие повторы точки и возвраты
/// p → q → p (OSM любит самокасания контуров) сворачиваются в одну точку.
/// Игла сама не видна, но кайма площадки рисует обе её стороны — светлый
/// шов «V» поверх газона обочины (Тула, сквер у Сойфера, N8).
pub(super) fn collapse_slivers(ring: &[Vec2]) -> Vec<Vec2> {
    let mut clean: Vec<Vec2> = Vec::with_capacity(ring.len());
    for &point in ring {
        if clean
            .last()
            .is_some_and(|last| last.distance(point) < RING_SLIVER)
        {
            continue;
        }
        // возврат в пред-предыдущую точку: вершина-игла между двумя копиями
        if clean.len() >= 2 && clean[clean.len() - 2].distance(point) < RING_SLIVER {
            clean.pop();
            continue;
        }
        clean.push(point);
    }
    // смыкание: кольцо идёт по кругу, последний и первый — та же точка
    while clean.len() > 1 && clean[0].distance(*clean.last().expect("len > 1")) < RING_SLIVER {
        clean.pop();
    }
    clean
}

fn push_area(map: &mut MapData, mut area: PolyArea) {
    area.outer = collapse_slivers(&area.outer);
    area.holes = area
        .holes
        .iter()
        .map(|hole| collapse_slivers(hole))
        .collect();
    if area.outer.len() < 3 {
        return;
    }
    match area.kind {
        AreaKind::Building | AreaKind::Kremlin => map.buildings.push(area),
        AreaKind::Water => map.water.push(area),
        AreaKind::Park => map.parks.push(area),
        AreaKind::Wood => map.woods.push(area),
        AreaKind::Grass => map.grass.push(area),
        AreaKind::Sand => map.sand.push(area),
        AreaKind::Residential | AreaKind::Industrial => map.landuse.push(area),
        AreaKind::Parking(_) => map.parking.push(area),
        AreaKind::Pitch(_) => map.pitches.push(area),
    }
}

fn parse_way(element: &Element, bounds: &GeoBounds, map: &mut MapData) {
    let Some(geometry) = &element.geometry else {
        return;
    };
    let points = project_points(geometry, bounds);
    if points.len() < 2 {
        return;
    }
    // `bridge=*`, кроме `no`, — одно правило для пути и для дороги
    let bridge = element
        .tags
        .get("bridge")
        .is_some_and(|value| value != "no");

    // Площадь дороги — первой и без `return`: `highway=*` + `area=yes` обязан
    // дойти и до дорожной ветки, где он был линией и до v15
    if let Some(kind) = road_area_kind(&element.tags)
        && let Some(outline) = as_ring(&points)
    {
        map.road_areas.push(RoadArea {
            outline,
            kind,
            osm_id: Some(element.id),
        });
    }

    // Рельсы проверяются до дорог и **не** прерывают разбор: трамвайный путь в
    // OSM сплошь и рядом висит на том же way, что и `highway=*`, и такой way
    // обязан стать и улицей, и путём.
    if let Some(railway) = element.tags.get("railway")
        && let Some((width, kind)) = rail_class(railway)
        && !is_underground(&element.tags)
    {
        map.rails.push(RailLine {
            points: points.clone(),
            width,
            kind,
            service: service_track(&element.tags),
            bridge,
            osm_id: Some(element.id),
        });
    }

    // Надземный трубопровод — теплотрасса на опорах. Тоже до дорог и тоже без
    // `return`: труба идёт эстакадой над улицей, и такой way в OSM носит оба
    // тега — стоя после дорожной ветки, она бы до него не добралась.
    if element.tags.get("man_made").map(String::as_str) == Some("pipeline")
        && let Some(width) = pipe_width(&element.tags)
    {
        map.pipes.push(PipeLine {
            points: points.clone(),
            width,
            osm_id: Some(element.id),
        });
    }

    // аллея — тоже до дорог и тоже без `return`, по той же причине, что рельсы:
    // ветка не должна затыкаться чужим ранним выходом
    if element.tags.get("natural").map(String::as_str) == Some("tree_row") {
        map.tree_rows.push(TreeRow {
            spacing: row_spacing(&element.tags, &points),
            radius: crown_radius(&element.tags),
            points: points.clone(),
        });
    }

    // водоток — снова до дорог и снова без `return`: ручей в трубе под улицей
    // размечен `waterway=*` на том же way, что и `highway=*`, и обе ветки обязаны
    // отработать. Замкнутый `waterway=riverbank` сюда не попадает — его нет в
    // белом списке, и площадью он остаётся ниже, в `area_kind`.
    if let Some(waterway) = element.tags.get("waterway")
        && let Some((default_width, kind)) = water_class(waterway)
    {
        map.water_lines.push(WaterLine {
            points: points.clone(),
            width: water_width(&element.tags).unwrap_or(default_width),
            kind,
            tunnel: is_underground(&element.tags),
            osm_id: Some(element.id),
        });
    }

    // ограда участка — снова до дорог и снова без `return`: обнесённый забором
    // квартал в OSM — это один way с `barrier=fence` **и** `landuse=residential`,
    // и он обязан стать и оградой, и кварталом (с `return` здесь Тула теряла три
    // площадки, три стоянки, квартал и парк), а забор вдоль тропы висит на том же
    // way, что и `highway=*`, и ниже дорожной ветки её `return` съедал бы его
    // целиком (в Париже и Лондоне по одному такому way, в остальных пяти городах
    // ни одного). Ограда — не стена: рисуется, но навмеша не касается.
    if let Some(kind) = fence_kind(&element.tags) {
        map.fences.push(FenceLine {
            points: points.clone(),
            kind,
            gates: Vec::new(),
            osm_id: Some(element.id),
        });
    }

    // подземное не рисуем — то же правило, что у рельсов и водотоков. У дорог
    // оно долго отсутствовало, и подземный переход выходил на карту обычной
    // дорожкой: в Токио так рисовались 1399 way из 12 859 (10.9%), в Лондоне
    // 1985, в Париже 1473, в Берлине 883, в Туле 35. Подавляющее большинство —
    // `highway=footway|steps` метрополитена. Что тег `tunnel` на этом way
    // может описывать вовсе не дорогу — вопрос [`is_road_underground`]
    if let Some(highway) = element.tags.get("highway") {
        let Some((width, class, highway)) = road_class(highway) else {
            return;
        };
        if is_road_underground(&element.tags) {
            return;
        }
        let passage = is_building_passage(&element.tags);
        // ширина дорожки — по тегу и виду; мостик и арка остаются при
        // ширине класса: их лента — коридор, который режет навмеш, и узкий
        // `width=1` на мостике сузил бы переход через реку до одного тайла
        let width = if class == RoadClass::Alley && !bridge && !passage {
            path_width(&element.tags)
        } else {
            width
        };
        // `oneway=-1` — поток против порядка точек; разворачиваем здесь, чтобы
        // ниже по конвейеру «направление way» и «направление движения» были
        // одним и тем же. Рельс и водоток той же ноды это не касается: их
        // ветки отработали выше и взяли `points` в исходном порядке
        let mut points = points;
        if is_oneway_backward(&element.tags) {
            points.reverse();
        }
        map.roads.push(RoadLine {
            points,
            width,
            class,
            highway,
            bridge,
            passage,
            oneway: is_oneway(&element.tags),
            roundabout: is_roundabout(&element.tags),
            lanes: tagged_lanes(&element.tags),
            lanes_backward: tagged_lanes_backward(&element.tags),
            parking_aisle: is_parking_aisle(&element.tags),
            turns: tagged_turns(&element.tags),
            lane_markings: has_lane_markings(&element.tags),
            sidewalks: tagged_sidewalks(&element.tags)
                .unwrap_or_else(|| untagged_sidewalks(&element.tags)),
            verges: [0.0; 2],
            verge_profile: Default::default(),
            parking: tagged_parking(&element.tags),
            pavement: tagged_pavement(&element.tags),
            osm_id: Some(element.id),
        });
        return;
    }

    if element.tags.get("barrier").map(String::as_str) == Some("city_wall") {
        map.walls.push(WallLine {
            points,
            width: WALL_WIDTH,
            osm_id: Some(element.id),
        });
        return;
    }

    // Цилиндр промзоны — резервуар, силос, труба, башня. Эта ветка, в
    // отличие от рельсовой и трубопроводной, **прерывает разбор**: труба,
    // размеченная way с
    // `building=yes` (в Туле такая одна из восьми), — тот же самый объект, и
    // коробка под цилиндром была бы им обоим сразу. Заодно у неё не
    // заводятся ни двери, ни навмеш-препятствие, чего трубе и не нужно.
    if let Some(structure) = parse_structure_way(element, &points) {
        map.structures.push(structure);
        return;
    }

    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(outer) = as_ring(&points) else {
        return;
    };
    // высота читается до того, как кольцо уедет в `PolyArea`: торговому этажу
    // нужен контур, а не только теги — и тот же класс, которым дом рисуется
    let building_use = area_use(kind, &element.tags);
    let height = area_height(kind, &element.tags, building_use, &outer);
    push_area(
        map,
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            building_use,
            height,
            storeys: area_storeys(kind, &element.tags),
            entrances: Vec::new(),
            colours: area_colours(kind, &element.tags),
            osm_id: Some(element.id),
        },
    );
}

/// Сторона движения — `driving_side` на административной границе, внутри
/// которой лежит центр карты (запрос `is_in` в `overpass_query`). На дорогах
/// тег почти не ставят: в OSM он живёт на стране и наследуется всем внутри.
///
/// Границ с тегом может прийти несколько (страна и заморская территория,
/// регион-исключение внутри страны) — берётся самая мелкая, с наибольшим
/// `admin_level`. Непонятное значение пропускается, как отсутствие тега.
fn driving_side(elements: &[Element]) -> Option<TrafficSide> {
    elements
        .iter()
        .filter(|element| element.kind == "relation")
        .filter_map(|element| {
            let side = match element.tags.get("driving_side")?.as_str() {
                "right" => TrafficSide::Right,
                "left" => TrafficSide::Left,
                _ => return None,
            };
            let level = element
                .tags
                .get("admin_level")
                .and_then(|level| level.parse::<u8>().ok())
                .unwrap_or(0);
            Some((level, side))
        })
        .max_by_key(|(level, _)| *level)
        .map(|(_, side)| side)
}

fn parse_relation(
    element: &Element,
    bounds: &GeoBounds,
    map: &mut MapData,
    skipped_open_rings: &mut usize,
) {
    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(members) = &element.members else {
        return;
    };

    let outers = assemble_rings(members, "outer", bounds, skipped_open_rings);
    let inners = assemble_rings(members, "inner", bounds, skipped_open_rings);
    let building_use = area_use(kind, &element.tags);
    let colours = area_colours(kind, &element.tags);
    // этажи — на relation, а не на кольцо: тег один на весь мультиполигон,
    // в отличие от высоты, которую торговой коробке меряет её собственное пятно
    let storeys = area_storeys(kind, &element.tags);

    for outer in outers {
        let holes = inners
            .iter()
            .filter(|inner| point_in_polygon(inner[0], &outer))
            .cloned()
            .collect();
        // высота — по каждому кольцу отдельно, а не одна на relation: этаж
        // торговой коробки меряется её пятном, и у ТЦ одним мультиполигоном
        // размечен и корпус, и пристройка под ним
        let height = area_height(kind, &element.tags, building_use, &outer);
        push_area(
            map,
            PolyArea {
                outer,
                holes,
                kind,
                building_use,
                height,
                storeys,
                entrances: Vec::new(),
                colours,
                osm_id: Some(element.id),
            },
        );
    }
}

/// Сборка замкнутых колец из way-членов relation с заданной ролью:
/// цепочки соединяются по совпадающим концам (с разворотом при
/// необходимости), пока не замкнутся.
fn assemble_rings(
    members: &[Member],
    role: &str,
    bounds: &GeoBounds,
    skipped_open_rings: &mut usize,
) -> Vec<Vec<Vec2>> {
    let mut segments: Vec<Vec<Vec2>> = members
        .iter()
        .filter(|member| member.kind == "way" && member.role == role)
        .filter_map(|member| member.geometry.as_ref())
        .map(|geometry| project_points(geometry, bounds))
        .filter(|points| points.len() >= 2)
        .collect();

    let close = |a: Vec2, b: Vec2| a.distance_squared(b) < RING_JOIN_EPSILON * RING_JOIN_EPSILON;
    let mut rings = Vec::new();

    while let Some(mut ring) = segments.pop() {
        loop {
            if close(ring[0], *ring.last().unwrap()) && ring.len() >= 4 {
                ring.pop();
                rings.push(ring);
                break;
            }

            let tail = *ring.last().unwrap();
            let Some(index) = segments.iter().position(|segment| {
                close(segment[0], tail) || close(*segment.last().unwrap(), tail)
            }) else {
                // не замкнулось (обычно порвано краем bbox): ≥3 точек —
                // насильно замыкаем, иначе выбрасываем
                if ring.len() >= 3 {
                    rings.push(ring);
                } else {
                    *skipped_open_rings += 1;
                }
                break;
            };

            let mut segment = segments.swap_remove(index);
            if !close(segment[0], tail) {
                segment.reverse();
            }
            ring.extend_from_slice(&segment[1..]);
        }
    }
    rings
}

mod block_pull;
mod faiths;
#[cfg(test)]
mod fixtures;
mod house_pull;
mod lots;
mod pavements;
mod pockets;
mod rails;
mod sidewalks;
mod squaring;
mod tags;
#[cfg(test)]
mod tests;
mod verges;

// Приватный реэкспорт: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать классификаторы.
use self::tags::{
    NON_WALKABLE_ENTRANCES, area_colours, area_height, area_kind, area_storeys, area_use,
    crown_radius, fence_kind, has_lane_markings, is_building_passage, is_oneway,
    is_oneway_backward, is_parking_aisle, is_road_underground, is_roundabout, is_underground,
    path_width, pipe_width, rail_class, road_area_kind, road_node_kind, row_spacing, service_track,
    structure_height, structure_kind, structure_radius, structure_size, tagged_lanes,
    tagged_lanes_backward, tagged_parking, tagged_pavement, tagged_sidewalks, tagged_turns,
    untagged_sidewalks, water_class, water_width,
};
// Словарь `highway` нужен и обрезке витрины (`osm::crop`): какие значения —
// дорожки, решает он один.
pub(in crate::map::osm) use self::tags::road_class;
// Доводочные проходы, разрезанные в соседние файлы: `finish_parse` зовёт их
// по-прежнему из корня, и снаружи дерево parse выглядит как и до разрезания.
use self::block_pull::{StretchedAreas, pull_areas_to_roads};
use self::house_pull::{PulledHouses, pull_houses_off_sidewalks};
use self::sidewalks::{InferredSidewalks, drop_untagged_sidewalks, infer_sidewalks};
use self::squaring::square_skewed_houses;
// Радиусы, по которым `tests` ходит в корень через `use super::*`: у корня
// самого их больше нет, и без этих строчек тестовая сборка их не увидит.
#[cfg(test)]
use self::house_pull::SIDEWALK_CLEARANCE;
#[cfg(test)]
use crate::map::osm::model::SidewalkSide;
