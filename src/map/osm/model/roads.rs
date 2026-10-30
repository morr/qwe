//! Дорожный словарь модели: классы и значения `highway`, сама [`RoadLine`] с
//! её тротуарным профилем и узловое сырьё этапов узла — [`RoadNode`] и
//! [`RoadArea`].

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadClass {
    Street,
    Alley,
}

/// Значение `highway` дороги — то, из чего выводится её сечение
/// (`map::roads::network::sections`) и по чему ways склеиваются в улицы: у
/// продолжения тот же класс. Съезды (`*_link`) — свой класс, а не класс
/// дороги, к которой они ведут: съезд уходит вбок, и продолжением главной
/// его считать нельзя.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Highway {
    Motorway,
    Trunk,
    Primary,
    Secondary,
    Tertiary,
    MotorwayLink,
    TrunkLink,
    PrimaryLink,
    SecondaryLink,
    TertiaryLink,
    Residential,
    Unclassified,
    LivingStreet,
    /// Дворовый проезд, въезд, проезд стоянки.
    Service,
    /// Всё аллейное (`footway|path|pedestrian|cycleway|steps|track`) — одним
    /// классом: сечения у дорожки нет, а склейка в улицы ей не нужна.
    Path,
}

impl Highway {
    /// Проезжая часть улицы: несёт тротуар, разметку и машины. Дворовый
    /// проезд — нет, как бы широк он ни был; дорожка — тем более.
    pub fn is_street(self) -> bool {
        !matches!(self, Self::Service | Self::Path)
    }

    pub fn is_link(self) -> bool {
        matches!(
            self,
            Self::MotorwayLink
                | Self::TrunkLink
                | Self::PrimaryLink
                | Self::SecondaryLink
                | Self::TertiaryLink
        )
    }

    /// Класс без связки: `X_link` → `X`, остальные как есть. Съезды от конца
    /// проспекта — связки его класса, и развилка на них — развилка той же
    /// дороги (Белгород, Победы, R34).
    pub fn family(self) -> Self {
        match self {
            Self::MotorwayLink => Self::Motorway,
            Self::TrunkLink => Self::Trunk,
            Self::PrimaryLink => Self::Primary,
            Self::SecondaryLink => Self::Secondary,
            Self::TertiaryLink => Self::Tertiary,
            other => other,
        }
    }
}

/// Дорога: осевая полилиния и ширина, выведенная из сечения
/// (`map::roads::network::sections`), у дорожек — по классу.
#[derive(Debug, Clone)]
pub struct RoadLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub class: RoadClass,
    /// Значение `highway` — см. [`Highway`].
    pub highway: Highway,
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
    /// Тег `junction=roundabout|circular`. Спрашивают не его, а
    /// [`RoadLine::is_roundabout`]: кольцо бывает и без тега.
    pub roundabout: bool,
    /// Число полос в обе стороны. Из разбора выходит тег (`lanes`, иначе
    /// `lanes:forward` + `lanes:backward`), а проход сечений
    /// (`map::roads::network::sections`) проставляет его каждой проезжей
    /// дороге и проезду: без тега — от соседей по улице, иначе по классу.
    /// `None` остаётся у дорожек и у дорог, собранных тестом руками; дефолт
    /// по ширине для них — у потребителя (`map::roads::lane_count`).
    pub lanes: Option<u8>,
    /// Сколько из [`Self::lanes`] двусторонней дороги идут **против** хода
    /// точек: `lanes:backward`, иначе `lanes` − `lanes:forward`. Где встаёт
    /// осевая: между потоками, а не посреди полотна, — при `lanes=5,
    /// lanes:forward=3` на границе третьей полосы. Проход сечений сверяет его
    /// с итоговым числом полос и дописывает участкам без тега от соседа по
    /// улице с тем же числом; `None` — не известно (делит потребитель,
    /// `roads::paint::axis_offset`) или дорога односторонняя.
    pub lanes_backward: Option<u8>,
    /// Проезд стоянки (`service=parking_aisle`) — полоса, по которой машина
    /// подъезжает к месту, и **единственная дорога, которую читает раскладка
    /// мест** (`map::parking`): ряды ложатся по обе стороны от неё. Рисуется
    /// как любой `service`, да и то под асфальтом самой стоянки.
    pub parking_aisle: bool,
    /// Манёвры полос по `turn:lanes`: `[по ходу точек, против]`, полосы слева
    /// направо по ходу движения. Пусто — тега нет. У односторонней — только
    /// первое (порядок точек и есть поток); у двусторонней общий `turn:lanes`
    /// без направления не читается — чьи это полосы, не сказано.
    pub turns: [Vec<LaneTurn>; 2],
    /// Разметка полос на проезжей части: `false` — `lane_markings=no`, у
    /// улицы нет ни осевой, ни границ полос (зебры и стоп-линии узла это не
    /// снимает). Без тега — `true`: есть ли линии, решает число полос.
    pub lane_markings: bool,
    /// Откуда у улицы тротуар `[слева, справа]` по ходу точек — см.
    /// [`SidewalkSide`]: по `sidewalk=*`, по классу и застройке, или его нет.
    /// Проезду и дорожке тротуар не положен и так
    /// (`map::roads::is_carriageway`), это поле его не добавляет.
    pub sidewalks: [SidewalkSide; 2],
    /// **Обочина** `[слева, справа]` по ходу точек, м: у стороны мощёной
    /// улицы, вдоль которой идёт отдельно замапленная мощёная дорожка, — от
    /// кромки до её оси (`parse/verges.rs::measure_footways_beside_streets`), с полосой
    /// тротуара или без; ноль — нет.
    /// Только рисунок: плитка под зеленью (`map::roads`), навмеш и дома её не
    /// читают.
    pub verges: [f32; 2],
    /// Обочина **по месту** `[слева, справа]`: `(метров по точкам, обочина)`
    /// у каждой пробы стороны, нашедшей дорожку, по возрастанию. Дорожка
    /// уходит от улицы и подходит к ней — у угла перекрёстка она
    /// заворачивает, — и обочина постоянной ширины (медиана [`Self::verges`])
    /// оставляла между собой и дорожкой клин голой земли (Тула, витрина 02).
    /// Пусто — обочина постоянная; читать через [`Self::verge_at`].
    pub verge_profile: [Vec<(f32, f32)>; 2],
    /// Стоянка у бордюра `[слева, справа]` по ходу точек — по `parking:*`.
    /// Что делать с [`KerbParking::Untagged`], решает правило
    /// (`map::roads::pockets`), не разбор.
    pub parking: [KerbParking; 2],
    /// Покрытие дорожки ([`RoadClass::Alley`]): мощёная рисуется плиткой
    /// тротуара, грунтовая — песчаной тропинкой. Из разбора выходит тег
    /// `surface` и то, что следует из вида дорожки (`footway=sidewalk`,
    /// `steps` — мощёные, `path`/`track` — грунтовые); `None` у дорожки без
    /// тега решает проход по окружению (`parse::pavements::infer_pavements`). У улицы
    /// это только тег `surface`: грунтовая ([`RoadLine::is_unpaved_street`])
    /// рисуется грунтом без разметки, без тега — асфальтом; `None` у
    /// дорожки, собранной тестом руками, рисуется тропинкой, как до поля.
    pub pavement: Option<Pavement>,
    /// OSM id элемента выгрузки; `None` — сгенерировано разбором или карта
    /// собрана тестом. Проба точки (`map/inspect.rs`) печатает его хвостом
    /// `#id` — от находки к тегам ведёт `tools/osm_near --id`.
    pub osm_id: Option<u64>,
}

impl RoadLine {
    /// Самая широкая проба обочины стороны: [`Self::verges`] против всех проб
    /// [`Self::verge_profile`] — по ней ступень зума решает, рисуется ли
    /// сторона вовсе: всплеск профиля выше медианы значит то же, что
    /// постоянная широкая обочина.
    pub fn verge_widest(&self, side: usize) -> f32 {
        self.verge_profile[side]
            .iter()
            .fold(self.verges[side], |widest, &(_, width)| widest.max(width))
    }

    /// Обочина стороны `side` в `along` метрах по точкам: по
    /// [`Self::verge_profile`] — между пробами по прямой, за крайними — как у
    /// крайней; без профиля — постоянная [`Self::verges`]; ноль — обочины нет.
    pub fn verge_at(&self, side: usize, along: f32) -> f32 {
        if self.verges[side] <= 0.0 {
            return 0.0;
        }
        let profile = &self.verge_profile[side];
        let (Some(&(first_at, first)), Some(&(last_at, last))) = (profile.first(), profile.last())
        else {
            return self.verges[side];
        };
        if along <= first_at {
            return first;
        }
        if along >= last_at {
            return last;
        }
        let next = profile.partition_point(|&(at, _)| at <= along);
        let ((from_at, from), (to_at, to)) = (profile[next - 1], profile[next]);
        let t = (along - from_at) / (to_at - from_at).max(f32::EPSILON);
        from + (to - from) * t
    }

    /// Грунтовая улица или проезд (`surface=unpaved|gravel|ground|dirt|…`):
    /// своим слоем под асфальтом, без линий краски и без стоп-линии по
    /// рангу узла (`map::roads`). Тротуара без тега у неё нет и так
    /// (`parse/tags.rs::untagged_sidewalks`).
    pub fn is_unpaved_street(&self) -> bool {
        self.class == RoadClass::Street && self.pavement == Some(Pavement::Unpaved)
    }

    /// Мощёная дорожка — рисуется в слое тротуаров его плиткой, а не
    /// песчаной тропинкой (`map::roads`).
    pub fn is_paved_path(&self) -> bool {
        self.class == RoadClass::Alley && self.pavement == Some(Pavement::Paved)
    }
}

/// Покрытие дорожки — см. [`RoadLine::pavement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pavement {
    /// Асфальт, плитка, бетон, брусчатка, настил — рисуется тротуаром.
    Paved,
    /// Грунт, гравий, утоптанная земля — песчаная парковая тропинка.
    Unpaved,
}

/// Откуда у одной стороны улицы тротуар — решение разбора, которое после него
/// не теряется: `sidewalk=no` и «снято по застройке» различимы по тому, был
/// ли тег.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidewalkSide {
    /// Тротуар по тегу `sidewalk*` (`both|left|right|yes`).
    Tagged,
    /// Тега нет — тротуар по правдоподобию: улица с твёрдым покрытием, и
    /// застройка вокруг не отняла его (`parse::infer_sidewalks`).
    Inferred,
    /// Тротуара нет: `sidewalk=no|none|separate` (отдельный footway рисуется
    /// сам), грунтовая улица без тега или застройка вокруг ниже городской.
    None,
}

impl SidewalkSide {
    /// Есть ли тротуар с этой стороны — неважно, откуда.
    pub fn is_present(self) -> bool {
        self != Self::None
    }
}

/// Стоянка вдоль одной стороны улицы по `parking:<side>` (схема 2022 г.).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KerbParking {
    /// Тега нет.
    #[default]
    Untagged,
    /// `lane`, `on_kerb`, `half_on_kerb`, `shoulder`, `yes` — машины у бордюра на
    /// проезжей части.
    Lane,
    /// `street_side` — карман: асфальт вне проезжей части, в тротуаре.
    Pocket,
    /// `no`, `separate`, или запрет остановки / стоянки
    /// (`parking:<side>:restriction=no_stopping|no_parking|no_standing`).
    No,
}

/// Куда можно из полосы (`turn:lanes`): `slight_*` и `sharp_*` — те же
/// поворот, `merge_to_*` и пустое значение — прямо, `reverse` — никуда.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LaneTurn {
    pub left: bool,
    pub through: bool,
    pub right: bool,
}

impl RoadLine {
    /// Кольцевая развязка — одностороннее кольцо, к которому улицы подходят
    /// торцами: **тег или форма**.
    ///
    /// Форма — потому что тега сплошь и рядом нет: в тульском кэше 12 way с
    /// `junction=roundabout` и ни одного замкнутого среди них (кольцо там
    /// разрезано на дуги), а большое кольцо у ТРЦ «Макси» (way 397005605) —
    /// наоборот, замкнутый `oneway=yes` без тега. По одному тегу оно
    /// получало две полосы разметки с износом — то, чего на кольце не рисуют,
    /// — и ряд припаркованных машин по кругу.
    ///
    /// Форма читается по **сырым** точкам OSM, а не по нарисованной оси:
    /// сглаживание концов не двигает, но зависеть от стиля рисования это
    /// свойство дороги не должно.
    pub fn is_roundabout(&self) -> bool {
        self.roundabout || (self.oneway && crate::map::shapes::is_ring(&self.points))
    }

    /// Прорезает ли дорога навмеш: мост (`bridge`) — коридор через воду, арка
    /// (`passage`) — сквозь дом. Точки такой дороги — навмеша, и рисовальщик
    /// её ось не трогает: не сглаживает, не стягивает в пару, не вписывает в
    /// кольцо, не режет под клин и не стежёт — мост кончается ровным срезом
    /// бордюра, арка приколота к стенам дома.
    pub fn carves_navmesh(&self) -> bool {
        self.bridge || self.passage
    }

    /// Проезжая часть улицы — то, что несёт тротуар и разметку и участвует в
    /// перекрёстках: улица по классу ([`Highway::is_street`] — не дворовый
    /// проезд), не арка (`passage` идёт сквозь дом). Мост — тоже: улица через
    /// реку не теряет полос.
    ///
    /// Решает **класс, а не ширина**: пока ширина шла по классу, порог в 8 м
    /// был тем же классом другими словами, но ширина из сечения
    /// (`network::sections`) у двухполосной улицы — 7.6 м, у однополосной
    /// односторонней — 4.3, и порог по ширине отнял бы у них тротуар.
    pub fn is_carriageway(&self) -> bool {
        self.class == RoadClass::Street && !self.passage && self.highway.is_street()
    }

    /// Тротуар дороги как он есть на карте — одно решение для разбора и
    /// рендера. Считается от текущей `width`, а не хранится: клоны переездов
    /// и дуг колец (`map::roads`) меняют ширину, и хранимая полоса устарела бы.
    pub fn sidewalk(&self) -> SidewalkProfile {
        SidewalkProfile {
            sides: self.sidewalks.map(SidewalkSide::is_present),
            band: self.is_carriageway().then(|| sidewalk_band(self.width)),
        }
    }
}

/// Доля ширины улицы на тротуар с каждой стороны и её пределы, м: у
/// магистрали в 16 м тротуар в 3 м, у жилой улицы в 8 м — 1.8 м.
const SIDEWALK_SHARE: f32 = 0.22;
pub const SIDEWALK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 1.2..=3.0;

/// Ширина тротуара с одной стороны проезжей части шириной `road_width`, м —
/// без вопроса, есть ли у дороги тротуар вообще (это [`RoadLine::sidewalk`]).
pub fn sidewalk_band(road_width: f32) -> f32 {
    (road_width * SIDEWALK_SHARE).clamp(*SIDEWALK_WIDTH_RANGE.start(), *SIDEWALK_WIDTH_RANGE.end())
}

/// Тротуар одной дороги ([`RoadLine::sidewalk`]): по каким сторонам он стоит
/// и какой ширины полоса положена ей по классу. Отсюда три края, которые
/// читает разбор, — у каждого свой довод, сливать их нельзя.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidewalkProfile {
    /// `[слева, справа]` по ходу точек — есть ли тротуар ([`SidewalkSide`]).
    sides: [bool; 2],
    /// Полоса по классу — у проезжей части ([`RoadLine::is_carriageway`]),
    /// мост включительно; у проезда и дорожки `None`.
    band: Option<f32>,
}

impl SidewalkProfile {
    /// Полоса по классу, тег не смотрит: край обочины у дома, бордюр стоянки.
    pub fn band(&self) -> Option<f32> {
        self.band
    }

    /// Тротуар с этой стороны `[слева, справа]` по ходу точек.
    pub fn on(&self, side: usize) -> Option<f32> {
        self.band.filter(|_| self.sides[side])
    }

    /// Тротуар хоть с одной стороны: у `sidewalk=no|separate` с обеих — нет.
    pub fn any(&self) -> Option<f32> {
        self.band.filter(|_| self.sides.contains(&true))
    }

    /// Тротуар с обеих сторон — по тегу или выведенный ([`SidewalkSide`]);
    /// ширину не спрашивает.
    pub fn both(&self) -> bool {
        self.sides == [true; 2]
    }

    /// `[слева, справа]` — есть ли тротуар по стороне.
    pub fn sides(&self) -> [bool; 2] {
        self.sides
    }

    /// **Край по карте** (mapped edge) — внешний край нарисованного полотна от
    /// оси: полуширина `half` плюс тротуар, если он есть хоть с одной
    /// стороны. Один на обе стороны и у одностороннего тротуара. К нему
    /// тянутся кварталы и стоянки (`parse::pull_areas_to_roads`,
    /// `parse/lots.rs`).
    pub fn mapped_edge(&self, half: f32) -> f32 {
        half + self.any().unwrap_or_default()
    }

    /// **Край обочины** (verge edge) — полоса по классу с зазором: от него
    /// отодвигается дом (`parse::pull_houses_off_sidewalks`). Тег не смотрит:
    /// там, где тег тротуар снял, остаётся обочина. `None` у проезда.
    pub fn verge_edge(&self, half: f32, clearance: f32) -> Option<f32> {
        self.band.map(|band| half + band + clearance)
    }

    /// Ширина бордюра, которым дорога сквозь стоянку отделена от её асфальта:
    /// полоса по классу (тег не смотрит), а у проезда без неё — `default`
    /// (`parking::LOT_KERB`).
    pub fn kerb(&self, default: f32) -> f32 {
        self.band.unwrap_or(default)
    }

    /// **Край бордюра** (kerb edge) — полуширина `half` плюс [`Self::kerb`]:
    /// ближе этого к оси место на стоянке не встаёт.
    pub fn kerb_edge(&self, half: f32, default: f32) -> f32 {
        half + self.kerb(default)
    }
}

/// Дорожный узел из OSM — точка на оси улицы, про которую данные знают больше,
/// чем про соседние: где зебра, где светофор, кто уступает, где тупик
/// расширен в площадку. Разбор его только читает; рисует по нему дорога:
/// краска узла — зебры, светофоры, стоп-линии (`map::roads::node_paint`),
/// разрывы ряда машин и карманов у перехода (`map::roads::pockets`),
/// разворотная площадка в тупике (`map::roads`). Мини-кольца и островки не
/// читает пока никто.
///
/// Позиция — там, где узел лежит в данных, то есть на оси way: переход — точка
/// пересечения оси тротуара с осью улицы, стоп-линию OSM ставит на ось перед
/// перекрёстком, площадку — на конец тупика.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadNode {
    pub pos: Vec2,
    pub kind: RoadNodeKind,
    /// OSM id элемента выгрузки; `None` — сгенерировано разбором или карта
    /// собрана тестом. Проба точки (`map/inspect.rs`) печатает его хвостом
    /// `#id` — от находки к тегам ведёт `tools/osm_near --id`.
    pub osm_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadNodeKind {
    /// `highway=crossing`. `signals` — `crossing=traffic_signals` или
    /// `crossing:signals=yes` (регулируемый переход); `island` —
    /// `crossing:island=yes` или `crossing=island` (островок посреди перехода);
    /// `marked` — снято только явным `crossing=unmarked` или
    /// `crossing:markings=no`: переход без тега вида (в Туле 111 из 801)
    /// считается размеченным, так правдоподобнее.
    Crossing {
        signals: bool,
        island: bool,
        marked: bool,
    },
    /// `highway=traffic_signals` — светофор узла, а не перехода.
    TrafficSignals,
    /// `highway=stop`.
    Stop,
    /// `highway=give_way`.
    GiveWay,
    /// `highway=mini_roundabout` — кольцо, нарисованное точкой.
    MiniRoundabout,
    /// `highway=turning_circle|turning_loop` — площадка разворота в тупике.
    TurningCircle,
    /// `traffic_calming=island` точкой — островок безопасности без контура.
    Island,
}

/// Площадь дороги, нарисованная маппером контуром: `area:highway=*` (покрытие
/// проезжей части или тротуара), `highway=*` + `area=yes` (площадь) и
/// `traffic_calming=island` контуром. Как и [`RoadNode`] — сырьё для этапов
/// узла, разбором не рисуется: `highway` + `area=yes` при этом по-прежнему
/// приходит и линией в [`MapData::roads`](super::MapData::roads), как приходил до v15.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadArea {
    /// Открытое кольцо (без повтора первой точки).
    pub outline: Vec<Vec2>,
    pub kind: RoadAreaKind,
    /// OSM id элемента выгрузки; `None` — сгенерировано разбором или карта
    /// собрана тестом. Проба точки (`map/inspect.rs`) печатает его хвостом
    /// `#id` — от находки к тегам ведёт `tools/osm_near --id`.
    pub osm_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadAreaKind {
    /// Проезжая часть: `area:highway` с классом улицы (`primary`, `service`,
    /// …) или `highway` такого класса с `area=yes`.
    Carriageway,
    /// Пешеходное: `footway`, `pedestrian`, `path`, `cycleway`, `steps` и
    /// прочее, что не проезжая часть.
    Walkway,
    /// Островок: `traffic_calming=island`, `area:highway=traffic_island`.
    Island,
}
