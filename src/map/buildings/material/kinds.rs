//! Словарь материалов: чем крыша покрыта ([`RoofKind`]), чем облицована стена
//! ([`WallKind`]), и классификация — по классу здания и посеву от его первой
//! вершины она выбирает материал и палитру. В рамку для шейдера выбор
//! оборачивают `roof_look`/`wall_look` родителя
//! ([`crate::map::meshing::ATTRIBUTE_ROOF`]).

use bevy::prelude::*;

use crate::map::buildings::classify::{BuildingClass, BuildingKind};
use crate::map::buildings::{fortress, temples};
use crate::map::osm::{BuildingUse, PolyArea, Sacred};

/// Чем крыша покрыта. Код материала (`code`) едет в вершинный атрибут и
/// разбирается шейдером; ноль занят «без фактуры» (оборудование кровли,
/// кайма), поэтому коды начинаются с единицы. Стена — не ноль и не кровля, а
/// продолжение того же словаря ([`WallKind`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofKind {
    /// Рулонный битумный ковёр: швы через метр, заплаты ремонта, лужи.
    /// Кровля почти всякого панельного и кирпичного дома.
    Bitumen,
    /// Гравийная засыпка по битуму — светлее и зернистее, без швов.
    Gravel,
    /// Фальцевый металл: частые рёбра с бликом, окрашенный или оцинковка.
    Seam,
    /// Профлист: волна много мельче фальца, гаражи и промка.
    Corrugated,
    /// Черепица, шифер, ондулин — ряды поперёк ската, частный сектор.
    Tile,
    /// ПВХ-мембрана: светлая, почти ровная, широкие полотнища. Новые ТЦ.
    Membrane,
    /// Лента гаражного кооператива: тот же профлист, но со швом на каждом
    /// боксе и своим тоном краски внутри шва. Ставится не по назначению, а
    /// по геометрии прогона — см. [`crate::map::buildings::garages`].
    GarageRow,
    /// Кооператив целиком одним контуром: та же лента, но с **проездами**
    /// между парами рядов — на снимке ГСК это сетка, а не полоса.
    GarageBlock,
}

impl RoofKind {
    /// Исчерпывающий список **материалов** — по нему идёт витрина
    /// `roof_gallery`. Гаражных лент тут нет: их выбирает не назначение дома
    /// с посевом, а геометрия прогона ([`crate::map::buildings::garages`]), и перебрать их
    /// витрина всё равно не может.
    pub const ALL: [Self; 6] = [
        Self::Bitumen,
        Self::Gravel,
        Self::Seam,
        Self::Corrugated,
        Self::Tile,
        Self::Membrane,
    ];

    /// Код для [`crate::map::meshing::ATTRIBUTE_ROOF`]; `0` — вершина без фактуры.
    ///
    /// Код **позиционный** — номер варианта плюс единица, — а его зеркало это
    /// набор констант `roof.wesl` (`BITUMEN = 1u` … `GARAGE_BLOCK = 8u`,
    /// дальше стены). Значит, вариант можно только дописать в конец: вставка в
    /// середину молча сдвинет коды всех, кто ниже, и шейдер начнёт класть
    /// чужую фактуру. Коды стен ([`WallKind::code`]) идут следом за последним
    /// кровельным, так что дописанная кровля сдвигает и их — зеркало в
    /// шейдере одно, и правится оно целиком.
    pub const fn code(self) -> u32 {
        self as u32 + 1
    }

    /// Сколько кодов занято кровлями. Считается по **последнему варианту**, а
    /// не по длине [`Self::ALL`], и это не мелочь: гаражные ленты в `ALL` не
    /// входят, а коды занимают, и стены, отсчитанные от длины списка, легли бы
    /// прямо на них.
    pub(crate) const CODES: u32 = Self::GarageBlock.code();

    pub fn label(self) -> &'static str {
        match self {
            Self::Bitumen => "Битум",
            Self::Gravel => "Гравий",
            Self::Seam => "Фальц",
            Self::Corrugated => "Профлист",
            Self::Tile => "Черепица",
            Self::Membrane => "Мембрана",
            // выбираются по геометрии прогона, а не по назначению, поэтому в
            // `ALL` их нет и витрина их не перебирает
            Self::GarageRow => "Гаражная лента",
            Self::GarageBlock => "Гаражный блок",
        }
    }

    /// Палитра материала: несколько правдоподобных цветов, дом выбирает свой
    /// посевом. Кремль и храм красятся не по материалу — их палитры знает
    /// [`palette`].
    pub fn palette(self) -> &'static [Color] {
        match self {
            Self::Bitumen => &BITUMEN_COLORS,
            Self::Gravel => &GRAVEL_COLORS,
            Self::Seam => &SEAM_COLORS,
            Self::Corrugated => &CORRUGATED_COLORS,
            Self::Tile => &TILE_COLORS,
            Self::Membrane => &MEMBRANE_COLORS,
            Self::GarageRow | Self::GarageBlock => &GARAGE_ROW_COLORS,
        }
    }
}

/// Чем облицована стена: не «из чего дом построен», а **что видно снаружи** и
/// как на этом расставлены проёмы. Материал решает и рисунок между окнами
/// (швы плит, ряды кирпича, рёбра профлиста), и сами окна, и бывают ли на
/// этой стене балконы.
///
/// Коды продолжают кровельные — один словарь в одном числе атрибута, — а
/// зеркало обоих половин лежит в `roof.wesl` (`PANEL = 9u` … `SACRED = 16u`,
/// за ними `DOOR = 17u`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WallKind {
    /// Панель: межэтажные швы, вертикальные швы плит, окно на панель и
    /// столбцы балконов. Типовая многоэтажка — то, чем была единственная
    /// стена до появления остальных.
    Panel,
    /// Кирпич: ряды кладки вместо швов, окно уже панельного, с перемычкой и
    /// отливом; балконы реже и утоплены — лоджией, а не выступом.
    Brick,
    /// Штукатурка: гладкая стена в пятнах, окна мелкие и редкие, балконов
    /// нет. Частный сектор, малоэтажная застройка, храм.
    Plaster,
    /// Витраж: сплошное остекление лентами через этаж, между ними глухой
    /// пояс, вертикальные импосты. Торговый центр, офис, новая общественная
    /// коробка.
    Shopfront,
    /// Профлист: вертикальные рёбра во всю стену, ленточное окно под
    /// карнизом да ворота внизу. Склад, промка, гаражный ряд.
    Shed,
    /// Композитная кассета: **глухая** стена крупными кассетами со швом между
    /// ними, ленточное остекление высоко под парапетом на части пролётов и
    /// тёмный цоколь. Гипермаркет и ТЦ **в три торговых уровня и ниже** —
    /// [`crate::map::osm::model::is_big_box`]; многоэтажный ТЦ в центре города
    /// носит витраж, а не кассету, и эту разницу предикат как раз и проводит.
    ///
    /// Отдельно от [`Shopfront`](Self::Shopfront) ровно потому, что это и есть
    /// разница между конторой и коробкой: у витража остекление лентами через
    /// этаж по всему фасаду, а у гипермаркета стекло — только у входа, всё
    /// прочее глухое, и ячейка у него не жилая панель в 3.2 м, а кассетный
    /// пролёт вдвое шире ([`layers::cell_width`] — 7 м). Пока торговая коробка
    /// одевалась жилыми таблицами, стодвадцатиметровый «Магнит» носил ряды
    /// квартирных окон.
    BigBox,
    /// Ворота: створка в **каждой** ячейке и ничего больше — ни окна, ни
    /// балкона. Стена гаражного прогона, и выбирается она не по назначению
    /// дома, а по геометрии — как и кровля прогона
    /// ([`crate::map::buildings::garages`]):
    /// ячейка тут не панель в 3.2 м, а сам бокс, поэтому створка приходится
    /// ровно под шов на кровле, и ряд ворот с рядом боксов говорят одно и то
    /// же.
    GarageDoors,
    /// Стена храма: ровная побелка или камень и **высокие арочные окна**
    /// редким рядом — ярус храма в шесть метров, а не жилой этаж в три, и
    /// обычное двухстворчатое окно на нём читалось бы жилым домом. Выбирается
    /// назначением (`BuildingUse::Church`), цвет — верой
    /// ([`crate::map::buildings::temples`]).
    Sacred,
}

impl WallKind {
    /// Исчерпывающий список — по нему идёт витрина `wall_gallery`.
    pub const ALL: [Self; 8] = [
        Self::Panel,
        Self::Brick,
        Self::Plaster,
        Self::Shopfront,
        Self::Shed,
        Self::BigBox,
        Self::GarageDoors,
        Self::Sacred,
    ];

    /// Код для [`crate::map::meshing::ATTRIBUTE_ROOF`]: продолжение кровельного словаря, поэтому
    /// он **выводится** из числа кровельных кодов ([`RoofKind::CODES`]), а не
    /// пишется числом. Дописанная кровля сдвинет коды стен — и это правильно:
    /// зеркало в `roof.wesl` одно на весь словарь и правится целиком.
    pub const fn code(self) -> u32 {
        RoofKind::CODES + 1 + self as u32
    }

    /// Сколько кодов занято облицовками — по **последнему** варианту, тем же
    /// правилом, что и [`RoofKind::CODES`]: от него отсчитывается дверь, и
    /// дописанная облицовка обязана её подвинуть, а не наехать на неё.
    pub(crate) const CODES: u32 = Self::Sacred.code();

    /// Стена ли это. Коды приходят из вершинного атрибута числом с плавающей
    /// точкой, и разбирать их порознь в тестах и в шейдере — верный способ
    /// разойтись; вопрос «стена или кровля» задаётся здесь.
    pub fn is_code(code: u32) -> bool {
        code >= Self::Panel.code() && code <= Self::CODES
    }
}

/// Код дверного полотна — последний в том же словаре, сразу за облицовками.
///
/// Дверь не облицовка и не кровля: это **отдельный четырёхугольник** поверх
/// стены, со своей рамой в клетку `[0, 1]²` ([`WallFrame::opening`]), и код ей
/// нужен ровно затем, чтобы шейдер узнал полотно и не искал в нём ни этажей,
/// ни панельных швов. Ставит её `buildings::layers::walls::push_doors` по входам из
/// `osm::entrances` — там же, где их видит гизмо дверей и куда идут пешки.
///
/// [`WallFrame::opening`]: crate::map::meshing::WallFrame::opening
pub const DOOR_CODE: u32 = WallKind::CODES + 1;

impl WallKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Panel => "Панель",
            Self::Brick => "Кирпич",
            Self::Plaster => "Штукатурка",
            Self::Shopfront => "Витраж",
            Self::Shed => "Профлист",
            Self::BigBox => "Кассета",
            Self::GarageDoors => "Ворота",
            Self::Sacred => "Храм",
        }
    }

    /// Палитра материала — как у кровель, дом выбирает свой цвет посевом.
    /// Кремль и храм красятся не по ней ([`wall_palette`]).
    pub fn palette(self) -> &'static [Color] {
        match self {
            Self::Panel => &PANEL_WALL_COLORS,
            Self::Brick => &BRICK_WALL_COLORS,
            Self::Plaster => &PLASTER_WALL_COLORS,
            Self::Shopfront => &SHOPFRONT_WALL_COLORS,
            Self::Shed => &SHED_WALL_COLORS,
            Self::BigBox => &BIG_BOX_WALL_COLORS,
            Self::GarageDoors => &GARAGE_WALL_COLORS,
            Self::Sacred => &temples::ORTHODOX_WALLS,
        }
    }
}

/// Палитры стен. Светлота выверена по тому же правилу, что и кровельная, но с
/// обратным знаком: **стена светлее своей крыши**, и это не стиль, а то, что
/// держит коробку 2.5D — тёмный битум над светлой панелью. Отсюда и границы:
/// панель и штукатурка живут в 0.66–0.86, кирпич опускается до 0.5 только на
/// тёмно-красном, и один лишь витраж заведомо темнее кровли — но он достаётся
/// торговым центрам, которых в квартале единицы.
///
/// Разброс внутри палитры — тот же приём, что у кровель: **много тона, мало
/// светлоты** у типовой застройки (соседние панельки отличаются оттенком) и
/// наоборот у частного сектора, где через забор стоят охра, белёный кирпич и
/// голубая штукатурка.
const PANEL_WALL_COLORS: [Color; 6] = [
    Color::srgb(0.78, 0.76, 0.71),
    Color::srgb(0.74, 0.73, 0.72),
    Color::srgb(0.80, 0.77, 0.68),
    Color::srgb(0.72, 0.74, 0.74),
    Color::srgb(0.76, 0.71, 0.66),
    Color::srgb(0.70, 0.72, 0.70),
];
const BRICK_WALL_COLORS: [Color; 6] = [
    // силикатный белый — половина тульских кирпичных домов
    Color::srgb(0.80, 0.79, 0.74),
    Color::srgb(0.74, 0.72, 0.66),
    // красный керамический и его выцветшие оттенки
    Color::srgb(0.63, 0.42, 0.34),
    Color::srgb(0.70, 0.50, 0.40),
    Color::srgb(0.55, 0.38, 0.32),
    Color::srgb(0.72, 0.62, 0.50),
];
const PLASTER_WALL_COLORS: [Color; 10] = [
    Color::srgb(0.86, 0.84, 0.78),
    Color::srgb(0.82, 0.75, 0.60),
    Color::srgb(0.78, 0.72, 0.66),
    Color::srgb(0.72, 0.76, 0.70),
    Color::srgb(0.70, 0.75, 0.80),
    Color::srgb(0.84, 0.72, 0.62),
    Color::srgb(0.66, 0.64, 0.60),
    // обшитый сайдингом дом: белый и светло-жёлтый, и потемневший брус
    Color::srgb(0.88, 0.88, 0.86),
    Color::srgb(0.84, 0.82, 0.64),
    Color::srgb(0.62, 0.50, 0.38),
];
const SHOPFRONT_WALL_COLORS: [Color; 4] = [
    Color::srgb(0.52, 0.56, 0.60),
    Color::srgb(0.60, 0.62, 0.64),
    Color::srgb(0.46, 0.52, 0.58),
    Color::srgb(0.64, 0.64, 0.62),
];
const SHED_WALL_COLORS: [Color; 5] = [
    Color::srgb(0.68, 0.69, 0.70),
    Color::srgb(0.56, 0.62, 0.70),
    Color::srgb(0.52, 0.60, 0.52),
    Color::srgb(0.72, 0.66, 0.56),
    Color::srgb(0.62, 0.60, 0.58),
];

/// Композитная кассета гипермаркета: с фотографии — белый, светло-серый и
/// песочный, и ровно один тёмный, «антрацит» новых ТЦ. Тон у самой стены
/// сдержанный **намеренно**: цвет коробки на снимке задаёт не она, а полоса
/// вывески поверх неё ([`layers::walls::push_brand_band`]), и яркая кассета спорила бы с
/// полосой вместо того, чтобы её держать.
const BIG_BOX_WALL_COLORS: [Color; 5] = [
    Color::srgb(0.86, 0.86, 0.85),
    Color::srgb(0.80, 0.80, 0.79),
    Color::srgb(0.82, 0.79, 0.73),
    Color::srgb(0.74, 0.75, 0.76),
    Color::srgb(0.42, 0.43, 0.45),
];

/// Стена гаражного прогона: побелка, силикатный кирпич, крашеный простенок
/// между воротами. Светлее самих створок, которые шейдер кладёт поверх, —
/// иначе ряд ворот не читается вовсе.
pub(super) const GARAGE_WALL_COLORS: [Color; 4] = [
    Color::srgb(0.74, 0.72, 0.67),
    Color::srgb(0.70, 0.68, 0.64),
    Color::srgb(0.72, 0.69, 0.62),
    Color::srgb(0.66, 0.66, 0.64),
];

/// Палитры материалов — по несколько правдоподобных цветов на каждый, дом
/// выбирает свой посевом. Светлота выверена по спутниковому снимку Тулы
/// **на общем плане**, где фактура уже погашена и от кровли остаётся один
/// базовый цвет: битум панельного дома там — средне-серый (~0.55 в sRGB),
/// а не почти чёрный; первая версия палитр (0.38–0.50) на светлой земле
/// карты (~0.9) читалась грязно-тёмными коробками.
///
/// Разброс внутри палитры разный по назначению. У **плоских кровель
/// корпусов** (битум, гравий, мембрана) он узкий: соседние панельные дома на
/// снимке отличаются оттенком, а не светлотой, и широкий разброс дал бы
/// конфетти. У **частного сектора** (черепица, а с ним фальц и профлист)
/// наоборот — цвета яркие и разные: красная и коричневая металлочерепица,
/// зелёная, синяя, серебристый и тёмный шифер стоят через забор друг от
/// друга, и палитра из серых оттенков делала посёлок одноцветным.
const BITUMEN_COLORS: [Color; 5] = [
    Color::srgb(0.55, 0.54, 0.52),
    Color::srgb(0.60, 0.58, 0.55),
    Color::srgb(0.50, 0.49, 0.48),
    Color::srgb(0.57, 0.55, 0.51),
    Color::srgb(0.63, 0.61, 0.58),
];
const GRAVEL_COLORS: [Color; 3] = [
    Color::srgb(0.70, 0.68, 0.64),
    Color::srgb(0.74, 0.72, 0.67),
    Color::srgb(0.66, 0.64, 0.60),
];
const SEAM_COLORS: [Color; 5] = [
    Color::srgb(0.62, 0.63, 0.64),
    Color::srgb(0.36, 0.52, 0.42),
    Color::srgb(0.60, 0.32, 0.27),
    Color::srgb(0.44, 0.50, 0.60),
    Color::srgb(0.70, 0.70, 0.69),
];
const CORRUGATED_COLORS: [Color; 5] = [
    Color::srgb(0.70, 0.71, 0.72),
    Color::srgb(0.42, 0.50, 0.62),
    Color::srgb(0.38, 0.52, 0.40),
    Color::srgb(0.62, 0.34, 0.28),
    Color::srgb(0.56, 0.40, 0.30),
];
/// Черепица частного сектора — металлочерепица всех цветов каталога: красная,
/// вишнёвая, шоколадная, зелёная, синяя, бирюзовая, серебристая и почти белая
/// оцинковка. На снимке с высоты посёлок пёстрый именно так.
const TILE_COLORS: [Color; 11] = [
    Color::srgb(0.72, 0.22, 0.18),
    Color::srgb(0.78, 0.45, 0.30),
    Color::srgb(0.58, 0.36, 0.26),
    Color::srgb(0.32, 0.50, 0.36),
    Color::srgb(0.30, 0.42, 0.64),
    Color::srgb(0.72, 0.71, 0.68),
    Color::srgb(0.45, 0.45, 0.44),
    Color::srgb(0.50, 0.20, 0.22),
    Color::srgb(0.38, 0.27, 0.21),
    Color::srgb(0.24, 0.50, 0.48),
    Color::srgb(0.80, 0.80, 0.78),
];
const MEMBRANE_COLORS: [Color; 3] = [
    Color::srgb(0.80, 0.80, 0.78),
    Color::srgb(0.75, 0.76, 0.76),
    Color::srgb(0.84, 0.84, 0.82),
];
/// Гаражная лента: оцинковка, шифер, крашеный суриком профлист — цвет один
/// на весь кооператив, разнобой боксов кладёт шейдер поверх.
///
/// Темнее прочих кровельных палитр на четверть, и это не вкус: на первом
/// закадровом снимке лента вышла почти белой полосой рядом с асфальтом в
/// 0.35. Гаражная кровля — это шифер и ржавая оцинковка, то есть тон между
/// битумом и асфальтом, а не свежий металл.
pub(super) const GARAGE_ROW_COLORS: [Color; 4] = [
    Color::srgb(0.45, 0.45, 0.44),
    Color::srgb(0.41, 0.39, 0.36),
    Color::srgb(0.40, 0.30, 0.25),
    Color::srgb(0.36, 0.39, 0.38),
];
/// Доли материалов по назначению — десять слотов, то есть проценты по
/// десяткам; дом берёт слот посевом. Не выдумка: по спутнику Тулы частный
/// сектор — черепица и шифер с вкраплениями профлиста, панельные кварталы —
/// сплошной битум, промзона — профлист.
const HOUSE_ROOFS: [RoofKind; 10] = [
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
];
const APARTMENTS_ROOFS: [RoofKind; 10] = [
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
    RoofKind::Membrane,
];
const COMMERCIAL_ROOFS: [RoofKind; 10] = [
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Corrugated,
];
/// Кровля гипермаркета: мягкая и **светлая**. На каждом из аэрофото это
/// мембрана или битум под гравийной присыпкой — почти белое поле, по которому
/// разбросано оборудование; фальца и профлиста на таком пролёте не бывает,
/// там мягкая кровля по профнастилу.
const BIG_BOX_ROOFS: [RoofKind; 10] = [
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Gravel,
    RoofKind::Bitumen,
];
/// Магазин у дома — обычная мелкая коробка: чаще всего наплавляемый битум,
/// у нового павильона фальц или профлист.
const SHOP_ROOFS: [RoofKind; 10] = [
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Membrane,
    RoofKind::Gravel,
];
const INDUSTRIAL_ROOFS: [RoofKind; 10] = [
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
];
const GARAGE_ROOFS: [RoofKind; 10] = [
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Seam,
    RoofKind::Tile,
];
const PUBLIC_ROOFS: [RoofKind; 10] = [
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Gravel,
    RoofKind::Gravel,
    RoofKind::Membrane,
];

/// Материал кровли этого дома: таблица по классу здания, развилки которого
/// разобрал классификатор. Кремль крыт металлом (его цвет всё равно свой),
/// частный сектор — по своей таблице, корпус — по жилой.
pub(super) fn kind_of(building: &PolyArea, class: &BuildingClass, seed: u32) -> RoofKind {
    let table = match class.kind() {
        BuildingKind::Fortress { .. } => return fortress::roof_kind(building),
        BuildingKind::Temple => return temples::roof_kind(sacred_of(building)),
        // частный сектор — дом и сарай двора кроются одинаково
        BuildingKind::House | BuildingKind::Shed => &HOUSE_ROOFS,
        BuildingKind::Garage => &GARAGE_ROOFS,
        BuildingKind::Hall => &INDUSTRIAL_ROOFS,
        BuildingKind::BigBox => &BIG_BOX_ROOFS,
        BuildingKind::Shop => &SHOP_ROOFS,
        // контора и торговый зал — одна конторская кровля
        BuildingKind::Office(_) | BuildingKind::Store => &COMMERCIAL_ROOFS,
        BuildingKind::Public => &PUBLIC_ROOFS,
        BuildingKind::Housing(_) | BuildingKind::DeepHull(_) => &APARTMENTS_ROOFS,
    };
    table[seed as usize % table.len()]
}

/// Палитра цвета кровли: Кремль и храм — своё, остальные по материалу.
pub(super) fn palette(
    building: &PolyArea,
    class: &BuildingClass,
    kind: RoofKind,
) -> &'static [Color] {
    match class.kind() {
        BuildingKind::Fortress { .. } => fortress::roof_palette(building),
        BuildingKind::Temple => temples::roof_palette(sacred_of(building).faith),
        _ => kind.palette(),
    }
}

/// Вера храма — данные `BuildingUse::Church`, которых у класса нет: класс
/// несёт развилки, вера читается владельцем. Классификатор ставит `Temple`
/// только храму, так что иначе ветка не бывает.
fn sacred_of(building: &PolyArea) -> Sacred {
    match building.building_use {
        BuildingUse::Church(sacred) => sacred,
        _ => unreachable!("BuildingKind::Temple comes only from BuildingUse::Church"),
    }
}

/// Материал стены этого дома: таблица по классу здания и его этажности.
///
/// Низкий дом не бывает ни панельным, ни витражным, чем бы он ни был по тегу:
/// у панели с балконами нет этажей, у витража — высоты, ради которой его
/// вешают. Всё ниже [`LOW_RISE_STOREYS`] уходит в малоэтажную таблицу —
/// **кроме** того, чей класс уже решил мимо роста: частный сектор, гараж,
/// торговые витрины (магазин узнают по витрине, а не по этажности — зал у
/// него один, и ниже порога он оказывается почти всегда), кассеты
/// гипермаркета и корпус глубже жилого.
///
/// Запрет на **панель** — не обещание, а устройство таблиц: панели нет ни в
/// [`BIG_BOX_WALLS`], ни в [`SHOP_WALLS`], так что низкий дом в панели не
/// встаёт и через классификатор.
pub(super) fn wall_kind_of(class: &BuildingClass, seed: u32) -> WallKind {
    let low = class.storeys() < LOW_RISE_STOREYS;
    let table: &[WallKind] = match class.kind() {
        BuildingKind::Fortress { .. } => return WallKind::Brick,
        BuildingKind::Temple => return WallKind::Sacred,
        // частный сектор идёт по своей таблице мимо развилки по росту: панели
        // среди его слотов нет
        BuildingKind::House | BuildingKind::Shed => &HOUSE_WALLS,
        BuildingKind::Garage => &GARAGE_WALLS,
        BuildingKind::Hall => &INDUSTRIAL_WALLS,
        BuildingKind::BigBox => &BIG_BOX_WALLS,
        BuildingKind::Shop => &SHOP_WALLS,
        // контора и торговый зал — до порога малоэтажности в штукатурку с
        // кирпичом, как всякая малоэтажка; витражом они достаются ростом
        BuildingKind::Office(_) | BuildingKind::Store => match low {
            true => &LOW_RISE_WALLS,
            false => &COMMERCIAL_WALLS,
        },
        BuildingKind::Public => match low {
            true => &LOW_RISE_WALLS,
            false => &PUBLIC_WALLS,
        },
        // корпус без назначения глубже любой жилой секции — не жильё, какой
        // бы высоты он ни был: ни панели, ни кирпича, а с ними и балконов
        // (`layers::walls::balcony_house`) у него нет
        BuildingKind::DeepHull(_) => &DEEP_HULL_WALLS,
        BuildingKind::Housing(_) => match low {
            true => &LOW_RISE_WALLS,
            false => &APARTMENTS_WALLS,
        },
    };
    table[seed as usize % table.len()]
}

/// Ниже скольких этажей дом считается малоэтажным, чем бы он ни был по тегу.
/// Тот же порог, по которому `layers.rs` не даёт стене балконов, — и это одна
/// константа, а не совпадающее число: одна граница, один смысл «панельного
/// дома тут нет».
const LOW_RISE_STOREYS: f32 = crate::map::buildings::layers::BALCONY_STOREYS_MIN;

/// Таблицы материалов стен — по десять слотов, чтобы читались как проценты,
/// ровно как кровельные.
const HOUSE_WALLS: [WallKind; 10] = [
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Shed,
];
const LOW_RISE_WALLS: [WallKind; 10] = [
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Shed,
    WallKind::Shopfront,
];
const APARTMENTS_WALLS: [WallKind; 10] = [
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
];
/// Стена корпуса без назначения, глубже жилого (класс
/// `BuildingKind::DeepHull` классификатора): профлист
/// цеха и склада, реже штукатурка старого корпуса. Ни панели, ни кирпича — это
/// те две стены, что носят балконы, а у цеха их не бывает.
const DEEP_HULL_WALLS: [WallKind; 10] = [
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
];
/// Стена гипермаркета: композитная кассета почти всегда, изредка окрашенный
/// профлист (строительный гипермаркет и оптовая база) и кирпич у старого
/// универсама. Витража тут нет вовсе — сплошное остекление торгового центра
/// это фасад ТРЦ в центре города, а не коробка у шоссе.
const BIG_BOX_WALLS: [WallKind; 10] = [
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::BigBox,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Brick,
];
/// Магазин у дома: витрина на весь первый этаж, у старого — штукатурка или
/// кирпич. Ровно то, чем встроенный магазин отличается от коробки в поле, — и
/// то, ради чего торговля проверяется до порога малоэтажности
/// ([`wall_kind_of`]): до появления [`BuildingUse::Retail`] магазин был
/// `Commercial`, то есть за порогом, и низкий уходил в [`LOW_RISE_WALLS`]
/// вместе с жилой двухэтажкой.
///
/// **Панели тут нет ни в одном слоте**, и это не вкус, а плата за тот самый
/// порядок: таблица разбирает дома ниже [`LOW_RISE_STOREYS`], а панель с её
/// межэтажными швами и столбцами балконов на двухэтажном павильоне и есть тот
/// жилой дом, которым торговля читаться не должна.
const SHOP_WALLS: [WallKind; 10] = [
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Shed,
];
const COMMERCIAL_WALLS: [WallKind; 10] = [
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Panel,
];
const PUBLIC_WALLS: [WallKind; 10] = [
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Panel,
];
const INDUSTRIAL_WALLS: [WallKind; 10] = [
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Panel,
];
const GARAGE_WALLS: [WallKind; 10] = [
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
];

/// Палитра цвета стены: Кремль и храм — своё, остальные по материалу. То же
/// правило и тот же порядок, что у [`palette`] для кровли.
pub(super) fn wall_palette(
    building: &PolyArea,
    class: &BuildingClass,
    kind: WallKind,
) -> &'static [Color] {
    match class.kind() {
        BuildingKind::Fortress { .. } => &fortress::WALL_COLORS,
        BuildingKind::Temple => temples::wall_palette(sacred_of(building).faith),
        _ => kind.palette(),
    }
}
