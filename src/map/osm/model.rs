//! Геометрическая модель карты в мировых метрах (юго-западный угол — (0,0)).

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    Building,
    Kremlin,
    Water,
    /// Парк — светлая подложка без деревьев (`leisure=park|garden`).
    Park,
    /// Лес внутри парка (`natural=wood` / `landuse=forest`) — единственное, что
    /// засаживается деревьями; рисуется темнее парка.
    Wood,
    /// Луг/газон: светлее парка и без деревьев (`landuse=grass|meadow`).
    Grass,
    /// Пляж или песчаная отмель (`natural=sand|beach`), тоже без деревьев.
    Sand,
    /// Жилой квартал (`landuse=residential`) — еле заметная тёплая заливка под
    /// всем остальным: подложка города перестаёт быть одним ровным листом.
    Residential,
    /// Промзона и гаражные кооперативы (`landuse=industrial|garages`) — та же
    /// подложка, но серее и холоднее жилья.
    Industrial,
    /// Стоянка (`amenity=parking`) — асфальт с расчерченными местами. На
    /// снимке двор со стоянкой ни с чем не спутать, и это единственная
    /// площадная зона, на которой что-то стоит (`map::cars`).
    Parking,
    /// Спортивная или детская площадка (`leisure=pitch|track|playground|…`).
    /// Вид спорта решает и цвет покрытия, и разметку, поэтому он едет прямо
    /// в значении: отдельного поля на `PolyArea` ради него заводить не за
    /// что — площадкой оно не бывает ни у чего другого.
    Pitch(PitchKind),
}

/// Что за площадка — по ней выбирается цвет покрытия и разметка
/// (`map::pitch`). Классов ровно столько, сколько различимо с воздуха.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PitchKind {
    /// Футбольное поле: газон или грунт, разметка белым.
    Soccer,
    /// Твёрдая площадка — баскетбол, волейбол, теннис, хоккейная коробка:
    /// асфальт или тартан, разметка по периметру и осевая.
    Hard,
    /// Беговая дорожка (`leisure=track`) — рыжий тартан, самое узнаваемое
    /// пятно во всём списке.
    Track,
    /// Детская площадка: песок и резиновая крошка, разметки нет.
    Playground,
    /// Спорткомплекс или стадион целиком — общая площадка, внутри которой
    /// лежат уже размеченные поля.
    Ground,
}

/// Назначение здания по `building=*` (и `amenity=*`, когда значение `building`
/// вне словаря — чаще всего просто `yes`) — класс **отрисовки**: у каждого
/// своя пара цветов крыши и стены, чтобы частный сектор, многоэтажки,
/// промзона и церкви читались с общего
/// плана. Не «вид опорного пункта» из `ROADMAP.md` — тот считается от
/// `amenity` своей веткой. Полный словарь значений — `parse/tags.rs::building_use`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildingUse {
    /// Частный дом: `house`, `detached`, `terrace`, `bungalow`, дача, изба.
    House,
    /// Многоквартирный: `apartments`, `residential`, `dormitory`, гостиница.
    Apartments,
    /// Торговля и офисы: `commercial`, `retail`, `office`, `kiosk`, супермаркет.
    Commercial,
    /// Промзона и склады: `industrial`, `warehouse`, `factory`, ангар, депо.
    Industrial,
    /// Гаражи, сараи, навесы: мелкие тёмные коробки.
    Garage,
    /// `building=garages` — **кооператив целиком** одним контуром, а не один
    /// бокс: в OSM так размечено большинство ГСК, и в Туле это пятна до
    /// 255 × 51 м. Отдельное значение, потому что рисуется оно не как здание,
    /// а как ряды боксов с проездами (`map/buildings/garages.rs`), и путать
    /// его с сараем нельзя.
    GarageBlock,
    /// Храм: `church`, `cathedral`, `chapel`, мечеть, синагога, `place_of_worship`.
    Church,
    /// Общественное: школа, больница, вуз, вокзал, музей, администрация.
    Public,
    /// `building=yes` и всё, чему пары цветов не назначено; у воды и парков
    /// тоже это значение.
    #[default]
    Other,
}

/// Полигон с дырками. Кольца открытые: последняя точка не повторяет первую.
#[derive(Debug, Clone)]
pub struct PolyArea {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
    pub kind: AreaKind,
    /// Назначение здания (`parse/tags.rs::building_use`); у всего, что не
    /// здание, — [`BuildingUse::Other`].
    pub building_use: BuildingUse,
    /// Высота здания в метрах из OSM (`parse::building_height`). `None` —
    /// тегов нет либо это не здание: у воды, парков и лугов высоты не бывает.
    /// Покрытие сильно зависит от города (Берлин 80%, Токио 5%), поэтому
    /// потребитель обязан иметь свой дефолт, а не считать `None` ошибкой.
    pub height: Option<f32>,
    /// Входы (`entrance=*`) на контуре этого здания. Пусто у подавляющего
    /// большинства домов и у всего, что не здание — потребитель обязан уметь
    /// работать без них. См. `parse::attach_entrances`.
    pub entrances: Vec<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadClass {
    Street,
    Alley,
}

/// Дорога: осевая полилиния и ширина по классу highway.
#[derive(Debug, Clone)]
pub struct RoadLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub class: RoadClass,
    /// `bridge=yes` — по такой дороге прорезается проходимый коридор через воду.
    pub bridge: bool,
    /// Арка: проезд/проход сквозь здание (`tunnel=building_passage`,
    /// `covered=building_passage|yes`). По такой дороге прорезается проходимый
    /// коридор сквозь уже заблокированное здание.
    pub passage: bool,
    /// Одностороннее движение (`oneway=*`, кольцо). У такой улицы нет осевой
    /// между встречными потоками — только границы полос, а машины паркуются
    /// одним рядом, справа по ходу. Порядок `points` **совпадает с
    /// направлением потока**: `oneway=-1` развёрнут при разборе.
    pub oneway: bool,
    /// Кольцевая развязка (`junction=roundabout|circular`): одностороннее
    /// кольцо, к которому улицы подходят торцами.
    pub roundabout: bool,
    /// `lanes` — число полос в обе стороны, если тег есть и правдоподобен.
    /// `None` — обычное дело; дефолт по ширине — у потребителя (`map::roads`).
    pub lanes: Option<u8>,
}

/// Род пути — он же способ отрисовки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    /// Магистральный путь: тёмная лента с белой штриховкой (osm-carto).
    Active,
    /// Трамвай: тонкая тёмно-красная линия с поперечными шпалами, как в
    /// Яндекс.Картах. Отдельный род, а не ширина, потому что рисуется он совсем
    /// другим примитивом — трамвай идёт по улице, и лента в пол-улицы шириной
    /// читалась бы как вторая дорога.
    Tram,
    /// Заброшенный: та же лента, что у [`Self::Active`], но выцветшая.
    Disused,
}

/// Ж/д путь: осевая полилиния и ширина по значению `railway`.
///
/// Навмеша не касается — слой чисто визуальный, люди ходят через пути как по
/// земле. Непрерывная линия, режущая город пополам, иначе отрезала бы половину
/// карты, и `prune_unreachable` её бы ампутировал.
#[derive(Debug, Clone)]
pub struct RailLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub kind: RailKind,
    /// Служебный путь (`service=siding|yard|spur|crossover`) — то есть
    /// станционный, а не главный ход. Рисуется он так же, но **вагоны стоят
    /// только на нём** (`map::wagons`): на главном ходу состав либо идёт,
    /// либо его там нет, а на подъездном он стоит неделями.
    pub service: bool,
}

/// Стена (Кремль): полилиния фиксированной ширины, непроходима.
#[derive(Debug, Clone)]
pub struct WallLine {
    pub points: Vec<Vec2>,
    pub width: f32,
}

/// Что за ограда — она же способ отрисовки (`map::fences`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceKind {
    /// `barrier=fence` — доска, штакетник, профнастил.
    Fence,
    /// `barrier=wall|retaining_wall` — бетон или кирпич, светлее и шире.
    Wall,
    /// `barrier=hedge` — живая изгородь, зелёная и мягкая.
    Hedge,
}

/// Ограда участка: полилиния и её род.
///
/// Отдельный тип, а не [`WallLine`] с полем: кремлёвская стена **непроходима
/// и попадает в навмеш**, а забор — декорация, которую пешки проходят
/// насквозь. Смешать их значило бы однажды перекрыть 427 линиями все дворы
/// частного сектора.
#[derive(Debug, Clone)]
pub struct FenceLine {
    pub points: Vec<Vec2>,
    pub kind: FenceKind,
}

/// Род промышленного сооружения — он же радиус и высота по умолчанию, когда в
/// данных нет ни контура, ни тегов размера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    /// `man_made=storage_tank` — резервуар: широкий низкий цилиндр, светлый
    /// металл. Самое узнаваемое круглое пятно нефтебазы.
    Tank,
    /// `man_made=silo` — силос: узкий и высокий, стоят батареями.
    Silo,
    /// `man_made=chimney` — труба: на снимке это маленький кружок и очень
    /// длинная тень, по которой её и узнают.
    Chimney,
    /// `man_made=water_tower` — водонапорная башня: бак на ноге. Ноги нет ни в
    /// геометрии, ни в тени — цилиндр сплошной от земли до бака, и тень у неё
    /// той же длины, что у трубы той же высоты.
    WaterTower,
    /// `man_made=gasometer` — газгольдер: круг вдвое шире резервуара.
    Gasometer,
}

/// Промышленное сооружение-цилиндр: где стоит, какого радиуса и какой высоты.
///
/// Контур не хранится: в OSM это либо нода, у которой контура нет вовсе, либо
/// почти всегда круглый way, и радиус по нему считается средним. Рисуется
/// цилиндр (`map/industry.rs`), и квадратная силосная башня выйдет круглой —
/// цена, которую видно только в упор.
#[derive(Debug, Clone, Copy)]
pub struct Structure {
    pub at: Vec2,
    pub radius: f32,
    pub height: f32,
    pub kind: StructureKind,
}

/// Надземный трубопровод — теплотрасса на опорах.
///
/// Отдельный тип, а не [`FenceLine`] с родом: ограда стоит на земле и делит
/// участки, труба идёт над землёй и через них перешагивает, отчего у неё и
/// тень длиннее, и слой выше. Подземные до карты не доезжают вовсе.
#[derive(Debug, Clone)]
pub struct PipeLine {
    pub points: Vec<Vec2>,
    /// Ширина связки, м: `count` труб в пучке — теплотрассу тянут парой
    /// (подача и обратка), реже четвёркой.
    pub width: f32,
}

/// Род водотока — он же ширина по умолчанию, когда в данных нет `width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterKind {
    /// `waterway=river` — русло реки, самое широкое из линейных.
    River,
    /// `waterway=canal` — канал, и `waterway=weir` (водослив поперёк русла):
    /// отдельным родом он не нужен, рисуется той же лентой и лежит внутри реки.
    Canal,
    /// `waterway=stream|brook` — ручей.
    Stream,
    /// `waterway=ditch|drain` — канава и дренаж, самые узкие.
    Ditch,
}

/// Линейный водоток (`waterway=river|stream|canal|…`): осевая полилиния и
/// ширина. Площадная вода — это [`AreaKind::Water`], а не эта структура:
/// `waterway=riverbank` замкнутым way по-прежнему становится полигоном.
///
/// В отличие от рельсов навмеш **трогает**: русло — та же вода, что пруд, и
/// перейти его можно только по мосту. От разрезания карты пополам спасают две
/// вещи — прорезка мостов после заливки и [`WaterLine::tunnel`].
#[derive(Debug, Clone)]
pub struct WaterLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub kind: WaterKind,
    /// Труба: `tunnel=culvert` у ручья под дорогой, коллектор под кварталом
    /// (`layer<0`) — то же `parse::is_underground`, что отсеивает метро у
    /// рельсов. Такой участок **не блокирует навмеш** (вода идёт под землёй,
    /// человек проходит поверху) и **не рисуется вовсе**: под землёй воды не
    /// видно, а карта и так показывает разрыв русла между порталами.
    pub tunnel: bool,
}

/// Допуск склейки концов двух ways: узел у них общий, и обе проекции считаны из
/// одной пары градусов — расходиться им негде, кроме младших битов `f32`.
const WATER_JOIN_EPSILON: f32 = 0.1;

/// Скруглены ли торцы русла — `[начало, конец]`. Скруглён тот конец, за
/// которым вода продолжается: два открытых way одного русла встречаются в общем
/// узле, и полудиски на их торцах сливают стык. **Портал культверта** — конец,
/// упирающийся в конец трубы, — срезан: за узлом воды уже нет, она ушла под
/// землю, а полудиск торчал бы на полуширину русла в сухую землю (и, поскольку
/// заливка навмеша меряет то же расстояние до отрезка, ещё и глушил бы вход в
/// культверт кругом непроходимых тайлов).
///
/// Правило одно на отрисовку (`spawn::mesh_water_lines`) и на заливку сетки
/// (`Navmesh::fill_from_mapdata`) — иначе слои разъедутся ровно на этот полудиск.
pub fn water_line_caps(line: &WaterLine, lines: &[WaterLine]) -> [bool; 2] {
    let joins_culvert = |point: Vec2| {
        lines
            .iter()
            .filter(|other| other.tunnel)
            .flat_map(|other| [other.points.first(), other.points.last()])
            .flatten()
            .any(|&end| end.distance_squared(point) <= WATER_JOIN_EPSILON * WATER_JOIN_EPSILON)
    };
    [line.points.first(), line.points.last()]
        .map(|end| !end.is_some_and(|&point| joins_culvert(point)))
}

/// Аллея из OSM (`natural=tree_row`): осевая полилиния и то, что данные знают о
/// самой посадке. Деревья по ней расставляет `planting::plant_rows`.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub points: Vec<Vec2>,
    /// Шаг посадки из тегов, м (`spacing`, либо длина / (`count` − 1)).
    /// `None` — в данных шага нет, и он берётся из ползунка плотности.
    ///
    /// Теги эти в OSM редки и полустандартны, так что почти каждый ряд —
    /// `None`; ветка «шаг из данных» проверяется тестом, а не городом.
    pub spacing: Option<f32>,
    /// Радиус кроны из `diameter_crown`, м. `None` — разыгрывается, как в лесу.
    pub radius: Option<f32>,
}

/// Одиночное дерево из OSM (`node natural=tree`): позиция и радиус кроны из
/// `diameter_crown`. `None` — радиус разыгрывается, как в лесу. Сажает
/// `planting::plant_standalone`; ноды в лесу и у аллей там же и отсеиваются.
#[derive(Debug, Clone, Copy)]
pub struct TreeNode {
    pub pos: Vec2,
    pub radius: Option<f32>,
}

/// Посаженное дерево: центр, радиус кроны и плотность, на которой оно
/// появляется (см. [`MapData::tree_appears_at`]).
pub type PlantedTree = (Vec2, f32, f32);

/// Что делать с деревом аллеи, попавшим на занятое место.
///
/// Живёт здесь, а не в `planting`, потому что по нему собирается
/// [`MapData::trees`]: политика — часть состояния модели, а не только аргумент
/// посадки. Переключается на лету из панели Tree rows (`TreeRowStyle::placement`),
/// поэтому оба варианта считаются на загрузке разом (см. `planting::plant_rows`).
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TreeRowPlacement {
    /// Позиция из OSM как есть: выбрасываются только деревья в домах и в воде.
    ///
    /// Именно так, потому что ширина дороги у нас **синтезирована**
    /// по классу highway (8–16 м) и о настоящих кромках ничего не знает.
    /// Аллея вдоль бульвара сплошь и рядом лежит внутри этой ширины, и полная
    /// проверка `blocked` стёрла бы ровно те ряды, ради которых всё и делалось.
    #[default]
    Keep,
    /// Занятое место — сдвиг вперёд по ряду до свободного; не нашлось на длине
    /// одного шага — дерева нет. Дороги и газоны при этом уважаются полностью.
    Slide,
}

impl TreeRowPlacement {
    pub const ALL: [Self; 2] = [Self::Keep, Self::Slide];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Slide => "slide",
        }
    }
}

/// Что решает, **где именно** встанут деревья аллеи: политика размещения и то,
/// слушаем ли мы шаг из тегов OSM. Обе оси меняют позиции, а не вид, поэтому
/// каждое сочетание раскладывается на загрузке и переключение в UI остаётся
/// пересборкой массива, а не пересадкой (см. [`RowTrees`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRowLayout {
    pub placement: TreeRowPlacement,
    /// `true` — шаг берётся из `spacing` / `count`, и ползунок плотности такой
    /// ряд не прореживает. `false` — теги игнорируются, ряд живёт по ползунку
    /// наравне с лесом.
    pub osm_spacing: bool,
}

impl Default for TreeRowLayout {
    fn default() -> Self {
        // по умолчанию данные важнее ползунка: если картограф проставил шаг,
        // он знает про эту аллею больше, чем наша формула
        Self {
            placement: TreeRowPlacement::default(),
            osm_spacing: true,
        }
    }
}

impl TreeRowLayout {
    pub const ALL: [Self; 4] = [
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: false,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: false,
        },
    ];
}

/// Аллеи, разложенные под каждое сочетание [`TreeRowLayout`]. Четыре варианта
/// вместо одного стоят копейки — рядов на карте сотни против десятков тысяч
/// лесных деревьев, — а взамен переключение любой из двух ручек не трогает
/// индексы близости, которые строятся по всем домам и дорогам карты.
#[derive(Debug, Default)]
pub struct RowTrees([Vec<PlantedTree>; 4]);

impl RowTrees {
    fn slot(layout: TreeRowLayout) -> usize {
        TreeRowLayout::ALL
            .iter()
            .position(|&known| known == layout)
            .expect("TreeRowLayout::ALL перечисляет все сочетания")
    }

    pub fn get(&self, layout: TreeRowLayout) -> &[PlantedTree] {
        &self.0[Self::slot(layout)]
    }

    pub fn set(&mut self, layout: TreeRowLayout, trees: Vec<PlantedTree>) {
        self.0[Self::slot(layout)] = trees;
    }
}

/// Что входит в собранный [`MapData::trees`]: раскладка аллей и какие
/// источники деревьев включены. Тумблеры панелей выключают источник целиком —
/// сборка та же, выключенное слагаемое пустое.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeCompose {
    pub layout: TreeRowLayout,
    /// Лесные массивы ([`MapData::wood_trees`]).
    pub woods: bool,
    /// Аллеи выбранной раскладки ([`MapData::row_trees`]).
    pub rows: bool,
    /// Одиночные деревья из OSM-нод ([`MapData::standalone_trees`]).
    pub standalone: bool,
}

impl Default for TreeCompose {
    fn default() -> Self {
        Self {
            layout: TreeRowLayout::default(),
            woods: true,
            rows: true,
            standalone: true,
        }
    }
}

/// Распарсенная карта; остаётся ресурсом после спавна — для отладки.
#[derive(Resource, Debug, Default)]
pub struct MapData {
    pub buildings: Vec<PolyArea>,
    pub water: Vec<PolyArea>,
    pub parks: Vec<PolyArea>,
    /// Лесные массивы — обычно внутри парков; только здесь растут деревья.
    pub woods: Vec<PolyArea>,
    /// Луга/газоны — лежат поверх парков и не засаживаются деревьями.
    pub grass: Vec<PolyArea>,
    /// Песчаные пляжи — тоже поверх парков и без деревьев.
    pub sand: Vec<PolyArea>,
    /// Кварталы `landuse` (жильё, промзона) — самая нижняя заливка, под
    /// парками; на навмеш и посадку деревьев не влияют.
    pub landuse: Vec<PolyArea>,
    /// Стоянки (`amenity=parking`) — асфальт с разметкой мест; по ним же
    /// расставляются машины. Навмеш не трогают: по стоянке ходят.
    pub parking: Vec<PolyArea>,
    /// Спортивные и детские площадки (`leisure=*`) — покрытие своего цвета и
    /// разметка (`map::pitch`). Навмеш не трогают: по площадке ходят.
    pub pitches: Vec<PolyArea>,
    pub roads: Vec<RoadLine>,
    /// Ж/д пути — только для отрисовки, в навмеш не попадают.
    pub rails: Vec<RailLine>,
    pub walls: Vec<WallLine>,
    /// Ограды участков (`barrier=fence|wall|hedge`) — только рисуются, навмеш
    /// не трогают: см. [`FenceLine`].
    pub fences: Vec<FenceLine>,
    /// Резервуары, силосы, трубы, башни (`man_made=*`) — тоже только рисуются:
    /// см. [`Structure`].
    pub structures: Vec<Structure>,
    /// Надземные трубопроводы (`man_made=pipeline`): см. [`PipeLine`].
    pub pipes: Vec<PipeLine>,
    /// Линейные водотоки — реки, ручьи, каналы, канавы. В навмеш попадают
    /// (кроме труб), в отличие от рельсов: см. [`WaterLine`].
    pub water_lines: Vec<WaterLine>,
    /// Аллеи (`natural=tree_row`) — исходная геометрия, для отладки; деревья по
    /// ним уже разложены в [`MapData::row_trees_kept`] / [`MapData::row_trees_slid`].
    pub tree_rows: Vec<TreeRow>,
    /// Одиночные деревья (`node natural=tree`) — сырые ноды, для отладки;
    /// посаженные лежат в [`MapData::standalone_trees`].
    pub tree_nodes: Vec<TreeNode>,
    /// Посаженные одиночные деревья, все с порогом 0 — дерево из данных видно
    /// всегда. Сырьё для [`MapData::compose_trees`], как и лес с аллеями.
    pub standalone_trees: Vec<PlantedTree>,
    /// Деревья лесных полигонов, по возрастанию порога появления. Сырьё для
    /// [`MapData::compose_trees`], а не то, что читает рендер.
    pub wood_trees: Vec<PlantedTree>,
    /// Деревья аллей под каждую раскладку, по возрастанию порога.
    pub row_trees: RowTrees,
    /// Из чего собран [`MapData::trees`]; `None` — ещё не собран.
    ///
    /// Признак живёт в `MapData`, а не в `Local` системы, именно потому, что при
    /// смене города ресурс заменяется целиком: `Local` пережил бы замену и
    /// решил, что для нового города всё уже собрано.
    pub composed_for: Option<TreeCompose>,
    /// Деревья карты: (центр, радиус). Детерминированы данными карты.
    /// Отсортированы по [`MapData::tree_appears_at`] — по возрастанию плотности,
    /// на которой дерево появляется. Собираются
    /// [`MapData::compose_trees`] из леса и аллей выбранной политики; всё
    /// остальное (рендер, тени, поле хвои, ползунок плотности) видит только этот
    /// массив и про аллеи не знает.
    pub trees: Vec<(Vec2, f32)>,
    /// На какой плотности (`TreeStyle::density`) появляется каждое дерево —
    /// массив **той же длины и того же порядка**, что [`MapData::trees`].
    ///
    /// Лес засаживается по потолку плотности, а ползунок показывает префикс:
    /// `trees[..partition_point(|d| d <= density)]`. Порог считается по номеру
    /// дерева внутри своего массива (`(номер + 1) · TREE_AREA_PER_TREE / площадь`),
    /// поэтому каждый лес отдаёт ровно свою долю, даже если он засаживался до
    /// упора и не добрал запрошенного (см. `planting.rs`).
    pub tree_appears_at: Vec<f32>,
}

impl MapData {
    /// Собрать [`MapData::trees`] из включённых источников: одиночные деревья,
    /// лес и аллеи выбранной раскладки.
    ///
    /// Все слагаемые уже отсортированы по порогу появления, так что это слияние,
    /// а не сортировка: префикс по плотности (`trees::visible_count`) обязан
    /// оставаться монотонным, иначе шаг ползунка вверх убирал бы деревья.
    pub fn compose_trees(&mut self, compose: TreeCompose) {
        // разбор по полям, а не `self.…`: аллеи читаются, пока выход пишется
        let MapData {
            row_trees,
            wood_trees,
            standalone_trees,
            trees,
            tree_appears_at,
            ..
        } = self;
        let empty: &[PlantedTree] = &[];
        let rows = if compose.rows {
            row_trees.get(compose.layout)
        } else {
            empty
        };
        let wood_trees: &[PlantedTree] = if compose.woods { wood_trees } else { empty };
        let standalone: &[PlantedTree] = if compose.standalone {
            standalone_trees
        } else {
            empty
        };

        trees.clear();
        tree_appears_at.clear();
        trees.reserve(standalone.len() + wood_trees.len() + rows.len());
        tree_appears_at.reserve(standalone.len() + wood_trees.len() + rows.len());

        // одиночные первыми: у всех порог 0, ниже любого лесного, и на равных
        // порогах с OSM-аллеями они и раньше стояли впереди
        for &(position, radius, at) in standalone {
            trees.push((position, radius));
            tree_appears_at.push(at);
        }

        let (mut wood, mut row) = (0, 0);
        while wood < wood_trees.len() || row < rows.len() {
            // при равных порогах первым идёт лес — порядок должен быть
            // детерминированным, иначе поле хвои и оттенки крон разъезжаются
            let take_wood = match (wood_trees.get(wood), rows.get(row)) {
                (Some(&(.., wood_at)), Some(&(.., row_at))) => wood_at <= row_at,
                (Some(_), None) => true,
                _ => false,
            };
            let &(position, radius, at) = if take_wood {
                wood += 1;
                &wood_trees[wood - 1]
            } else {
                row += 1;
                &rows[row - 1]
            };
            trees.push((position, radius));
            tree_appears_at.push(at);
        }

        self.composed_for = Some(compose);
    }
}

/// Точка внутри кольца (even-odd raycast). Кольцо открытое.
pub fn point_in_polygon(point: Vec2, ring: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Внутри внешнего кольца и вне всех дырок.
pub fn point_in_area(point: Vec2, area: &PolyArea) -> bool {
    point_in_polygon(point, &area.outer)
        && !area.holes.iter().any(|hole| point_in_polygon(point, hole))
}

/// Ближайшая точка отрезка. Проекция, зажатая концами: за пределами отрезка
/// ближайшая точка — его конец, а не точка на прямой.
pub fn closest_on_segment(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return from;
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    from + segment * t
}

/// Расстояние от точки до отрезка.
pub fn distance_to_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    point.distance(closest_on_segment(point, from, to))
}

/// Длина ломаной — сумма её звеньев.
pub fn polyline_length(points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum()
}

/// Точка на ломаной в `distance` метрах от начала. За концом — последняя точка:
/// ставить что-либо дальше ломаной некуда, и обрыв на её конце — единственный
/// разумный ответ (так расставляются деревья ряда и так ищется середина арки).
///
/// Звенья нулевой длины пропускаются: делить на них нечего, а в OSM-геометрии
/// после округления координат они встречаются.
pub fn point_at_arc_length(points: &[Vec2], distance: f32) -> Vec2 {
    let mut walked = 0.0;
    for segment in points.windows(2) {
        let (from, to) = (segment[0], segment[1]);
        let length = from.distance(to);
        if length <= 0.0 {
            continue;
        }
        if walked + length >= distance {
            return from.lerp(to, (distance - walked) / length);
        }
        walked += length;
    }
    *points
        .last()
        .expect("вызывается только для points.len() >= 2")
}

/// Знаковая площадь кольца по формуле шнурования: положительная — обход
/// против часовой стрелки. Знак нужен тени (свипы силуэта обязаны быть
/// одинаково закручены) и генератору входов (от обхода зависит, куда смотрит
/// внешняя нормаль грани), поэтому базовая формула — знаковая, а абсолютная
/// [`ring_area`] получается из неё.
pub fn signed_ring_area(ring: &[Vec2]) -> f32 {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += ring[j].perp_dot(ring[i]);
        j = i;
    }
    doubled / 2.0
}

/// Площадь кольца, абсолютная.
pub fn ring_area(ring: &[Vec2]) -> f32 {
    signed_ring_area(ring).abs()
}

/// AABB кольца: (min, max).
pub fn ring_bounds(ring: &[Vec2]) -> (Vec2, Vec2) {
    let mut min = Vec2::INFINITY;
    let mut max = Vec2::NEG_INFINITY;
    for &point in ring {
        min = min.min(point);
        max = max.max(point);
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ]
    }

    #[test]
    fn point_in_polygon_square() {
        let ring = square();
        assert!(point_in_polygon(Vec2::new(5.0, 5.0), &ring));
        assert!(!point_in_polygon(Vec2::new(15.0, 5.0), &ring));
        assert!(!point_in_polygon(Vec2::new(-1.0, 5.0), &ring));
    }

    #[test]
    fn point_in_area_respects_holes() {
        let area = PolyArea {
            outer: square(),
            holes: vec![vec![
                Vec2::new(4.0, 4.0),
                Vec2::new(6.0, 4.0),
                Vec2::new(6.0, 6.0),
                Vec2::new(4.0, 6.0),
            ]],
            kind: AreaKind::Building,
            building_use: BuildingUse::Other,
            height: None,
            entrances: Vec::new(),
        };
        assert!(point_in_area(Vec2::new(2.0, 2.0), &area));
        assert!(!point_in_area(Vec2::new(5.0, 5.0), &area));
    }

    #[test]
    fn ring_area_square() {
        assert!((ring_area(&square()) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn polyline_length_sums_the_links() {
        let path = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
        assert!((polyline_length(&path) - 7.0).abs() < 1e-4);
    }

    #[test]
    fn point_at_arc_length_walks_across_a_corner() {
        let path = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
        assert!(point_at_arc_length(&path, 1.5).distance(Vec2::new(1.5, 0.0)) < 1e-4);
        // ровно на изломе и за ним — второе звено
        assert!(point_at_arc_length(&path, 5.0).distance(Vec2::new(3.0, 2.0)) < 1e-4);
    }

    /// За концом обрыв, а не экстраполяция: расставлять что-либо дальше
    /// ломаной некуда.
    #[test]
    fn point_at_arc_length_stops_at_the_end() {
        let path = [Vec2::ZERO, Vec2::new(3.0, 0.0)];
        assert_eq!(point_at_arc_length(&path, 99.0), Vec2::new(3.0, 0.0));
    }

    /// Звено нулевой длины пропускается, а не делит на ноль — в OSM-геометрии
    /// после округления координат такие встречаются.
    #[test]
    fn point_at_arc_length_skips_a_zero_length_link() {
        let path = [Vec2::ZERO, Vec2::ZERO, Vec2::new(4.0, 0.0)];
        assert!(point_at_arc_length(&path, 2.0).distance(Vec2::new(2.0, 0.0)) < 1e-4);
    }
}
