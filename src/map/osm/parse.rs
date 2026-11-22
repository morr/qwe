//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов. Деревья по разобранной карте сажает
//! `super::planting`.

use bevy::math::Vec2;
use bevy::prelude::{Reflect, ReflectDefault, ReflectResource, Resource};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::city::City;
use crate::grid::DEFAULT_NAVTILE_SIZE;
use crate::map::osm::model::{MapData, TrafficSide, TreeRowLayout};
use crate::map::osm::overpass::{GeoBounds, LatLon, OverpassResponse};
use crate::map::roads::network::sections::SectionReport;
use crate::map::roads::rings;
use crate::map::roads::shape::LANE_WIDTH_DEFAULT;

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
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// Разошёлся ли снимок с ручками, с которыми мир должен быть разобран:
    /// условие перезагрузки мира (`city::parse_knobs_diverged`) сравнивает
    /// осевшие ручки с этим снимком. Сравнение — у снимка, а не у условия:
    /// список полей, по которым мир перезагружается, — это список полей
    /// снимка, и новая ручка разбора подхватывается им сама.
    pub fn diverged(&self, current: &ParseKnobs) -> bool {
        self != current
    }
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
/// - [`RawOsm::Draw`] — то же, и ещё снят весь набор достроек отрисовки
///   ([`DrawBuildout::ALL`]): дорога — простая лента по оси OSM без скруглений,
///   склейки узлов, пар половин, разделительных, устьев, газонов обочин и
///   разметки (`roads::mesh_raw_roads`); стоянка и площадка — своим полигоном
///   без раскладки мест и разметки поля; ни машин, ни стоящих вагонов
///   (`wagons::mesh_wagons`), ни сгенерированных деревьев (лес и аллеи не
///   сажаются, остаются только деревья-ноды OSM).
#[derive(
    Resource,
    Reflect,
    SettingsGroup,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
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
    pub(crate) fn skips_passes(self) -> bool {
        self != Self::Off
    }

    /// Выключена ли достройка отрисовки `what`. Второй уровень снимает весь
    /// набор ([`DrawBuildout::ALL`]): ответ «какие достройки выключены»
    /// держит модель, а не потребитель — ветка называет свою достройку, и
    /// новый член набора попадает под прицел сама, коль скоро её потребитель
    /// спрашивает здесь.
    pub fn disables(self, what: DrawBuildout) -> bool {
        self == Self::Draw && DrawBuildout::ALL.contains(&what)
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

/// Достройка отрисовки — то, что игра рисует сверх данных, чего в данных нет.
/// Перечень здесь — целостность второго уровня сырого OSM: [`RawOsm::Draw`]
/// снимает весь набор ([`Self::ALL`]), и потребитель называет свою достройку
/// в вопросе ([`RawOsm::disables`]), а не держит личную ветку «мы — наша
/// достройка». Новый член набора — вариант здесь, строка в [`Self::ALL`] и
/// правка его потребителя на этот вопрос; перечисленные в доке
/// [`RawOsm::Parse`] проходы — другой набор, свой у `finish_parse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawBuildout {
    /// Полная сборка дорог — склейка узлов, пары половин, разделительные,
    /// устья, газоны обочин и разметка; на сыром OSM простая лента по оси
    /// OSM (`roads::mesh_raw_roads`).
    DecoratedRoads,
    /// Разметка мест стоянок и разметка полей площадок; сама раскладка мест
    /// при этом не считается (`spawn::mesh_surfaces`).
    LotMarkings,
    /// Ряды припаркованных машин.
    ParkedCars,
    /// Стоящие вагоны.
    StandingWagons,
    /// Посаженные деревья — лес и аллеи; остаются деревья-ноды OSM.
    PlantedTrees,
}

impl DrawBuildout {
    /// Весь набор достроек отрисовки — то, что снимает [`RawOsm::Draw`].
    pub const ALL: &'static [Self] = &[
        Self::DecoratedRoads,
        Self::LotMarkings,
        Self::ParkedCars,
        Self::StandingWagons,
        Self::PlantedTrees,
    ];
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
    /// Вершин в снимке контуров OSM
    /// ([`OsmContours`](crate::map::osm::contours::OsmContours)) и цена копии.
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

mod block_pull;
mod elements;
mod faiths;
mod finish;
#[cfg(test)]
mod fixtures;
mod house_pull;
// `pub(crate)`: порог большой стоянки берёт сцена-фиксстура дорог
// (`roads/fixtures.rs`), чтобы тесты не отстали от парсинга
pub(crate) mod lots;
mod pavements;
mod pockets;
mod rails;
mod relations;
mod sidewalks;
mod squaring;
mod tags;
#[cfg(test)]
mod tests;
mod verges;
mod ways;

// Приватный реэкспорт: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать классификаторы
// — и соседние файлы разбира way/relation зовут их через корень.
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
// Элементная и финишная половины разбора, разрезанные в соседние файлы:
// `parse_response` зовёт их по-прежнему из корня, как и `elements` — `ways`
// с `relations`, а `finish` — проходы из своих соседей.
use self::block_pull::{StretchedAreas, pull_areas_to_roads};
use self::elements::{Pending, parse_structure_way, read_elements};
use self::finish::{finish_parse, vertex_key, vertex_uses};
// Тесты зовут их через корень (`squaring/tests.rs` — по имени, `tests.rs` —
// глобом `use super::*`); в обычной сборке корень их не трогает.
#[cfg(test)]
use self::finish::{attach_entrances, drop_buildings_in_water};
use self::house_pull::{PulledHouses, pull_houses_off_sidewalks};
use self::relations::{as_ring, parse_relation, push_area};
use self::sidewalks::{InferredSidewalks, drop_untagged_sidewalks, infer_sidewalks};
use self::squaring::square_skewed_houses;
use self::ways::{driving_side, parse_way};
// Радиусы, по которым `tests` ходит в корень через `use super::*`: у корня
// самого их больше нет, и без этих строчек тестовая сборка их не увидит.
#[cfg(test)]
use self::house_pull::SIDEWALK_CLEARANCE;
#[cfg(test)]
use crate::map::osm::model::{SidewalkSide, TreeCompose, point_in_area};
