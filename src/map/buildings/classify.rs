//! Что это за здание — одна классификация на все вопросы.
//!
//! Назначение из OSM ([`BuildingUse`]), пятно и пороги отвечают не на один
//! вопрос, а на четыре: сколько у дома этажей, чем он крыт, чем облицован и
//! что стоит на его кровле. До классификатора каждый вопрос разбирал развилки
//! сам — и условия разъезжались: кровля мелкой тегированной коробки без
//! назначения верила пятну, стены — ещё и этажам, и девятиэтажка на 200 м²
//! выходила с панельными стенами под черепичной двускатной. Теперь развилки
//! разобраны **один** раз — [`classify`] — и каждый вопрос читает готовый
//! ответ: высотные таблицы — [`heights`], палитры стен и крыш —
//! [`material`], оборудование кровли — [`clutter`].
//!
//! Вариант [`BuildingKind`] называет то, что видно на аэрофото: частный
//! сектор, панельная секция, гипермаркет, цех, храм, крепость. Данные, которых
//! у вида нет (вера храма, тегированная высота), остаются в
//! [`PolyArea`](crate::map::osm::PolyArea) — класс несёт развилки, а не
//! содержимое; вера и форма храма живут в `BuildingUse::Church` и читаются
//! владельцем ([`temples`]).

use bevy::math::Vec2;

use super::HouseFootprint;
use super::heights::inferred_height;
use super::layers::BALCONY_STOREYS_MIN;
use super::material::building_seed;
use super::roofs::{SHED_FOOTPRINT_MAX, SMALL_FOOTPRINT_MAX};
use crate::map::meshing::min_area_rect;
use crate::map::osm::model::{is_big_box, is_fortress_tower, signed_ring_area};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea, Sacred, SacredForm};
use crate::settings::STOREY_HEIGHT;

/// Чем здание оказалось, когда все развилки разобраны.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BuildingKind {
    /// Крепость: башня под шатром или прясло стены. Назначение у башни —
    /// `building=yes`, поэтому развилка по [`AreaKind::Kremlin`] и форме.
    Fortress {
        /// Башня ли ([`is_fortress_tower`]), а не прясло стены.
        tower: bool,
    },
    /// Храм любой веры: высота, кровля и облицовка ведутся верой и формой
    /// (`temples`), классификатор отвечает только за то, что это храм.
    Temple,
    /// Гаражный бокс — отдельный или кусок прогона; прогон решает выше.
    Garage,
    /// Частный дом: `building=house`, любого размера и любой тегированной
    /// высоты — балконам он запрещён отдельно ([`layers::balcony_house`]).
    House,
    /// Сарай частного сектора — мелкая коробка без назначения.
    Shed,
    /// Жилой или прочий корпус по форме пятна.
    Housing(Shape),
    /// Корпус без назначения глубже жилого ([`DEEP_HULL_MIN`]) — склад, НИИ,
    /// а не жильё, какой бы высоты он ни был: ни панели, ни кирпича, а с ними
    /// и балконов ([`layers::balcony_house`]).
    DeepHull(Shape),
    /// Контора: мелкая коммерция. Облицовка и кровля по назначению, высота —
    /// по форме пятна, как у жилья.
    Office(Shape),
    /// Гипермаркет / ТЦ: оболочка над одним высоким залом.
    BigBox,
    /// Магазин у дома — отдельно стоящий павильон.
    Shop,
    /// Торговый зал: крупная коммерция без признаков крупноформата.
    Store,
    /// Цех, склад: один высокий пролёт.
    Hall,
    /// Казённое здание — школа, поликлиника, контора: этажность по корпусу,
    /// облицовка своя.
    Public,
}

/// Форма пятна, по которой корпус выводит свою этажность. Решает пятно, как
/// его читает глаз на аэрофото; у тегированного здания форма не читается —
/// высоту несёт тег.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    /// Длинная узкая лента — панельная секция.
    Section,
    /// Компактное крупное пятно — башня.
    Tower,
    /// Всё прочее крупное — кирпичный корпус, школа, контора.
    Block,
    /// Мелкий контур — старый малоэтажный дом.
    Low,
}

/// Класс здания: чем оно оказалось, какая у него высота и сколько этажей.
///
/// Высота и этажность считаются при классификации один раз — тег из OSM,
/// когда он есть, иначе вывод вида, — и дальше читаются готовыми: высоту
/// берут тени и слой машин, этажность — стены со своими рядами окон.
pub(super) struct BuildingClass {
    kind: BuildingKind,
    height: f32,
    storeys: f32,
    pitched: bool,
}

impl BuildingClass {
    /// Чем здание оказалось.
    pub(super) fn kind(&self) -> BuildingKind {
        self.kind
    }

    /// Высота дома для отрисовки: тег из OSM, иначе вывод вида.
    pub(super) fn height(&self) -> f32 {
        self.height
    }

    /// Этажность фасада — высота, поделённая на ярус вида и округлённая. Та
    /// же, которой стены считают свои ряды окон.
    pub(super) fn storeys(&self) -> f32 {
        self.storeys
    }

    /// Скатная когорта: частный сектор без двора. На дворе конёк не встаёт —
    /// двускатная крыша рисуется над прямоугольником пятна, и двор его рвёт.
    pub(super) fn is_pitched(&self) -> bool {
        self.pitched
    }

    /// Ландмарк — крепость или храм: над ними стоит своё (главы, зубцы,
    /// шатры), и оборудование кровли генератора на них не сажают.
    pub(super) fn is_landmark(&self) -> bool {
        matches!(self.kind, BuildingKind::Fortress { .. } | BuildingKind::Temple)
    }
}

/// Класс здания — единственный разбор развилок «назначение × пятно × пороги».
///
/// `fp` — пятно дома ([`HouseFootprint`]), посчитанное сборщиком слоя один
/// раз на дом: ветка формы и корпус-глубина читают его без нового перебора
/// рёбер. `None` — пятна нет, ветка посчитает его сама, только если дойдёт;
/// так зовут класс вне сборки слоя (слой машин, тесты). Значение класс
/// **не** зависит от `fp`: и форма, и глубина отвечают то же самое по своему
/// перебору.
pub(super) fn classify(building: &PolyArea, fp: Option<HouseFootprint>) -> BuildingClass {
    // вид по назначению и пятну — без высоты: у нетегированного частного
    // сектора вывод и так малоэтажный (частный дом — один этаж с чердаком),
    // и гвард этажности ниже решает только тегированные коробки
    let mut kind = kind_of(building, fp);
    // тег всегда сильнее вывода
    let height = building
        .height
        .unwrap_or_else(|| inferred_height(kind, building_seed(building), building));
    // этажность фасада — высота, поделённая на ярус вида: у храма и торговой
    // коробки ярус свой, у прочих — жилой этаж
    let storeys = (height / facade_tier(building)).round().max(1.0);
    // Мелкая коробка без назначения, доросшая до корпуса, — не частный
    // сектор. Частный дом по определению малоэтажный; ниже
    // [`BALCONY_STOREYS`] тегированный дом ещё им остаётся, выше — это
    // точечный корпус, и стены давно одевают его панелью. Теперь и кровля
    // со скатной когортой согласны с ними: одна развилка, один ответ.
    if matches!(kind, BuildingKind::House | BuildingKind::Shed)
        && building.building_use == BuildingUse::Other
        && storeys >= BALCONY_STOREYS_MIN
    {
        kind = BuildingKind::Housing(Shape::Low);
    }
    let pitched = matches!(kind, BuildingKind::House | BuildingKind::Shed)
        && building.holes.is_empty();
    BuildingClass {
        kind,
        height,
        storeys,
        pitched,
    }
}

/// Единственный матч классификатора: назначение × пятно × пороги.
///
/// Порядок веток повторяет тот, по которому стены разбирали дом (рост —
/// после назначений с фиксированной облицовкой): частный сектор и гараж
/// одеваются по своему назначению мимо развилки по этажам, торговля
/// делится размером, и лишь прочее крупное идёт по форме пятна.
fn kind_of(building: &PolyArea, fp: Option<HouseFootprint>) -> BuildingKind {
    // крепость меряется не назначением (у башни это `building=yes`), а тем,
    // стена это или башня
    if building.kind == AreaKind::Kremlin {
        return BuildingKind::Fortress {
            tower: is_fortress_tower(building),
        };
    }
    let area = signed_ring_area(&building.outer).abs();
    match building.building_use {
        BuildingUse::Garage | BuildingUse::GarageBlock => BuildingKind::Garage,
        BuildingUse::House => BuildingKind::House,
        BuildingUse::Church(_) => BuildingKind::Temple,
        BuildingUse::Industrial => BuildingKind::Hall,
        // торговля делится размером, а не тегом: у гипермаркета мягкое светлое
        // поле в фонарях и вентиляции, у магазина у дома — обычная мелкая
        // кровля, и общая таблица стирала бы ровно эту разницу
        BuildingUse::Retail if is_big_box(building) => BuildingKind::BigBox,
        BuildingUse::Retail => BuildingKind::Shop,
        // торговля меряется пятном и здесь: гипермаркет — оболочка над одним
        // высоким залом, магазин у дома — павильон, а крупная коммерция без
        // признаков крупноформата — торговый зал
        BuildingUse::Commercial if area >= STORE_FOOTPRINT_MIN => BuildingKind::Store,
        BuildingUse::Public => BuildingKind::Public,
        // мелочь без назначения — постройки частного сектора: сарай во дворе
        // и сам дом, а не старая двух-трёхэтажка. Граница дома — та же, по
        // которой `roofs` кроет его двускатной, а `material` одевает стенами
        // дома: одна граница, один смысл
        BuildingUse::Other if area <= SHED_FOOTPRINT_MAX => BuildingKind::Shed,
        BuildingUse::Other if area <= SMALL_FOOTPRINT_MAX => BuildingKind::House,
        // корпус без назначения глубже любой жилой секции — не жильё, какой
        // бы высоты он ни был
        BuildingUse::Other if is_deep_hull(building, fp) => {
            BuildingKind::DeepHull(shape_of(building, fp))
        }
        // контора: облицовка и кровля по назначению, высота — по форме
        BuildingUse::Commercial => BuildingKind::Office(shape_of(building, fp)),
        // жильё и половина города без назначения — по форме пятна
        _ => BuildingKind::Housing(shape_of(building, fp)),
    }
}

/// Ярус фасада, которым высота делится на этажи: у колокольни звенья выше
/// храмовых, у торговой коробки ярус — торговый зал целиком, у прочих домов —
/// жилой этаж.
fn facade_tier(building: &PolyArea) -> f32 {
    match building.building_use {
        BuildingUse::Church(Sacred {
            form: SacredForm::Tower,
            ..
        }) => BELL_TOWER_TIER,
        BuildingUse::Church(_) => SACRED_TIER,
        BuildingUse::Retail if is_big_box(building) => BIG_BOX_TIER,
        _ => STOREY_HEIGHT,
    }
}

/// Форма пятна по тем же правилам, что и раньше вела вывод этажности:
/// **площадь спрашивается первой** — контур до [`LOW_FOOTPRINT_MAX`] старый
/// малоэтажный дом, даже если он лента по сторонам (40 × 7 м — сарай, а не
/// панельная секция).
fn shape_of(building: &PolyArea, fp: Option<HouseFootprint>) -> Shape {
    let area = signed_ring_area(&building.outer).abs();
    if area <= LOW_FOOTPRINT_MAX {
        return Shape::Low;
    }
    let (length, width) = fp
        .map(HouseFootprint::size)
        .unwrap_or_else(|| footprint_size(&building.outer));
    let squarish = width > 0.0 && length / width <= TOWER_MAX_RATIO;
    if length >= SLAB_MIN_LENGTH && width <= SLAB_MAX_WIDTH {
        Shape::Section
    } else if area >= TOWER_FOOTPRINT_MIN && squarish {
        Shape::Tower
    } else {
        Shape::Block
    }
}

/// Длина и ширина пятна по его кольцу — для тех, кому пятно не передали:
/// стороны минимального описанного прямоугольника, длинная первой. Тот же
/// `min_area_rect`, что считает [`super::HouseFootprint`] для сборки слоя.
fn footprint_size(ring: &[Vec2]) -> (f32, f32) {
    let Some(rect) = min_area_rect(ring) else {
        return (0.0, 0.0);
    };
    ((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length())
}

/// Длина пятна, от которой оно читается как секция, и предельная ширина
/// такой секции, м. Панельный дом — это лента 12–16 м в ширину и от сорока в
/// длину; всё, что шире, уже корпус.
const SLAB_MIN_LENGTH: f32 = 35.0;
const SLAB_MAX_WIDTH: f32 = 18.0;
/// Пятно, ниже которого корпус считается старым малоэтажным, м².
const LOW_FOOTPRINT_MAX: f32 = 300.0;
/// Башней считается пятно не меньше этого и не вытянутее этого отношения
/// сторон: у настоящей башни план почти квадратный.
const TOWER_FOOTPRINT_MIN: f32 = 500.0;
const TOWER_MAX_RATIO: f32 = 1.7;
/// Пятно, выше которого «торговое» здание — это зал, а не контора, м².
const STORE_FOOTPRINT_MIN: f32 = 800.0;

/// Ярус колокольни, м: семидесятиметровая колокольня в шестиметровых ярусах
/// носила двенадцать рядов окон, как башня-жильё.
pub(super) const BELL_TOWER_TIER: f32 = 12.0;
/// Ярус храмовой стены, м: высокое арочное окно на трёхметровом этаже не
/// встаёт, и храм в 18 м читался бы шестиэтажкой.
pub(super) const SACRED_TIER: f32 = 6.0;
/// Ярус стены гипермаркета, м: торговый зал с техэтажом над ним — это один
/// ярус фасада.
pub(super) const BIG_BOX_TIER: f32 = 5.5;

/// Глубина плана, начиная с которой дом без назначения — корпус, а не жильё, м.
/// Жилая секция глубиной 12–18 м, точечный дом — до ~25: свет в квартиры идёт
/// с двух фасадов, и глубже жилья не строят. Дальше — цех, склад, НИИ, ТЦ.
pub(super) const DEEP_HULL_MIN: f32 = 30.0;

/// Корпус глубже жилого ([`DEEP_HULL_MIN`]) — по **обеим** мерам глубины сразу:
///
/// * короткая сторона минимального описанного прямоугольника (`min_area_rect`,
///   тот же, по которому идёт конёк и длинная ось плана) — габарит;
/// * короткая сторона **равновеликого прямоугольника** — того, у которого те же
///   площадь и периметр, что у плана (с дворами: их площадь вычтена, их
///   периметр прибавлен). У сплошного прямоугольника она равна габариту, а у
///   каре, буквы Г или П из жилых крыльев — ширине крыла.
///
/// Одного габарита мало, и это измерено: квартал-каре или Г-образный дом в
/// 15 м шириной даёт габарит в 60–100 м, и одним им в «корпуса» ушли бы ещё
/// 258 домов Орла и 240 Тулы с `building=yes` — ровно жилые. Порог по площади
/// (> 3000 м²) отвергнут по той же причине с другой стороны: он ловит длинные
/// жилые пластины 20–29 м шириной, у которых глубина жилая.
///
/// Если равновеликого прямоугольника нет (план компактнее квадрата — круг,
/// восьмиугольник), глубина — сторона квадрата того же периметра. Пятно дома
/// приходит готовым ([`HouseFootprint`]); `None` — пятна нет, оно считается
/// на месте, как раньше.
pub(super) fn is_deep_hull(building: &PolyArea, fp: Option<HouseFootprint>) -> bool {
    let Some(fp) = fp.or_else(|| HouseFootprint::of(building)) else {
        return false;
    };
    let (length, width) = fp.size();
    let span = length.min(width);
    span > DEEP_HULL_MIN && equivalent_width(building) > DEEP_HULL_MIN
}

/// Короткая сторона прямоугольника с той же площадью и тем же периметром, что у
/// плана: корень `w² − (P/2)·w + A = 0`. Площадь — за вычетом дворов, периметр —
/// с их контурами; сам корень считает общая с шириной парковочной полосы
/// [`crate::map::meshing::equivalent_width`].
fn equivalent_width(building: &PolyArea) -> f32 {
    let area = signed_ring_area(&building.outer).abs()
        - building
            .holes
            .iter()
            .map(|hole| crate::map::osm::model::signed_ring_area(hole).abs())
            .sum::<f32>();
    let perimeter = crate::map::meshing::ring_perimeter(&building.outer)
        + building
            .holes
            .iter()
            .map(crate::map::meshing::ring_perimeter)
            .sum::<f32>();
    crate::map::meshing::equivalent_width(perimeter, area)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn building(
        outer: Vec<Vec2>,
        height: Option<f32>,
        kind: AreaKind,
        building_use: BuildingUse,
    ) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            building_use,
            height,
            storeys: None,
            entrances: Vec::new(),
            colours: Default::default(),
            osm_id: None,
        }
    }

    fn oblong(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::ZERO,
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    fn square() -> Vec<Vec2> {
        oblong(10.0, 10.0)
    }

    fn plain(use_: BuildingUse) -> PolyArea {
        building(oblong(14.0, 40.0), None, AreaKind::Building, use_)
    }

    fn tagged(use_: BuildingUse, height: f32) -> PolyArea {
        building(oblong(14.0, 40.0), Some(height), AreaKind::Building, use_)
    }

    /// Таблица соответствий «здание → класс»: одна строка на ветку
    /// классификатора — добавил ветку, добавь строку. Класс — то, чем здание
    /// оказалось после единственного разбора развилок; по нему высота, кровля,
    /// стены и оборудование кровли обязаны отвечать согласованно.
    #[test]
    fn the_building_lands_in_the_class_the_eye_expects() {
        let cases: &[(&str, PolyArea, BuildingKind)] = &[
            // жильё — по форме пятна
            ("панельная секция", plain(BuildingUse::Apartments), BuildingKind::Housing(Shape::Section)),
            ("башня", building(oblong(26.0, 30.0), None, AreaKind::Building, BuildingUse::Apartments), BuildingKind::Housing(Shape::Tower)),
            ("кирпичный корпус", building(oblong(24.0, 60.0), None, AreaKind::Building, BuildingUse::Apartments), BuildingKind::Housing(Shape::Block)),
            ("малоэтажка", building(oblong(12.0, 20.0), None, AreaKind::Building, BuildingUse::Apartments), BuildingKind::Housing(Shape::Low)),
            // частный сектор — по назначению и пятну
            ("частный дом любого размера", building(oblong(8.0, 600.0), None, AreaKind::Building, BuildingUse::House), BuildingKind::House),
            ("тегированный частный дом остаётся домом", tagged(BuildingUse::House, 27.0), BuildingKind::House),
            ("мелкая коробка без назначения — частный дом", building(square(), None, AreaKind::Building, BuildingUse::Other), BuildingKind::House),
            ("сарай двора", building(oblong(3.0, 3.0), None, AreaKind::Building, BuildingUse::Other), BuildingKind::Shed),
            // тегированная мелкая коробка, доросшая до корпуса, — не частный
            // сектор: тот редкий случай, где тег высоты побеждает пятно
            ("девятиэтажка на мелкой коробке — корпус", building(oblong(12.0, 20.0), Some(27.0), AreaKind::Building, BuildingUse::Other), BuildingKind::Housing(Shape::Low)),
            ("низкая мелкая коробка с тегом — частный дом", building(oblong(12.0, 20.0), Some(6.0), AreaKind::Building, BuildingUse::Other), BuildingKind::House),
            // торговля — по пятну, а не по тегу
            ("гипермаркет", building(oblong(80.0, 110.0), None, AreaKind::Building, BuildingUse::Retail), BuildingKind::BigBox),
            ("магазин у дома", building(oblong(14.0, 20.0), None, AreaKind::Building, BuildingUse::Retail), BuildingKind::Shop),
            ("торговый зал", building(oblong(40.0, 90.0), None, AreaKind::Building, BuildingUse::Commercial), BuildingKind::Store),
            ("контора", building(oblong(26.0, 30.0), None, AreaKind::Building, BuildingUse::Commercial), BuildingKind::Office(Shape::Tower)),
            // цех, казённое, гараж — по назначению
            ("цех", building(oblong(40.0, 90.0), None, AreaKind::Building, BuildingUse::Industrial), BuildingKind::Hall),
            ("казённое здание", building(oblong(28.0, 32.0), None, AreaKind::Building, BuildingUse::Public), BuildingKind::Public),
            ("гаражный бокс", building(oblong(6.0, 24.0), None, AreaKind::Building, BuildingUse::Garage), BuildingKind::Garage),
            ("гаражный блок", plain(BuildingUse::GarageBlock), BuildingKind::Garage),
            // корпус глубже жилого — по обеим мерам глубины
            ("корпус без назначения 130 × 145", building(oblong(130.0, 145.0), Some(27.0), AreaKind::Building, BuildingUse::Other), BuildingKind::DeepHull(Shape::Tower)),
            // храм и крепость — вне стилизации по назначению
            ("храм", plain(BuildingUse::Church(Sacred { faith: crate::map::osm::Faith::Orthodox, form: SacredForm::Nave, complex: 0, floor_dm: 0 })), BuildingKind::Temple),
            ("крепостная башня", building(square(), None, AreaKind::Kremlin, BuildingUse::Other), BuildingKind::Fortress { tower: true }),
            ("крепостная стена", building(oblong(3.0, 40.0), None, AreaKind::Kremlin, BuildingUse::Other), BuildingKind::Fortress { tower: false }),
        ];
        for (name, building, expected) in cases {
            let class = classify(building, None);
            assert_eq!(class.kind(), *expected, "{name}");
        }
    }

    /// Этажность фасада считается один раз — тег делится на ярус вида; у
    /// жилого этажа это и есть этажность стен.
    #[test]
    fn storeys_come_from_the_height_over_the_kind_tier() {
        let tower = tagged(BuildingUse::Apartments, 27.0);
        assert_eq!(classify(&tower, None).storeys(), 9.0);
        // торговая коробка делит на свой ярус: восемь метров — один ярус
        let big_box = building(oblong(80.0, 110.0), Some(8.0), AreaKind::Building, BuildingUse::Retail);
        assert_eq!(classify(&big_box, None).storeys(), 1.0);
        // этажность не бывает нулевой: одноэтажный сарай — один этаж
        let shed = building(oblong(3.0, 3.0), Some(2.0), AreaKind::Building, BuildingUse::Other);
        assert_eq!(classify(&shed, None).storeys(), 1.0);
    }

    /// Скатная когорта — частный сектор без двора; тегированная коробка,
    /// доросшая до корпуса, из неё выпадает вместе с домом-корпусом.
    #[test]
    fn the_pitched_cohort_is_the_private_sector_without_a_courtyard() {
        let house = plain(BuildingUse::House);
        assert!(classify(&house, None).is_pitched());
        let mut yard = house.clone();
        yard.holes.push(oblong(4.0, 4.0));
        assert!(!classify(&yard, None).is_pitched(), "a courtyard has no ridge");
        assert!(classify(&building(square(), None, AreaKind::Building, BuildingUse::Other), None).is_pitched());
        let tall_box = building(oblong(12.0, 20.0), Some(27.0), AreaKind::Building, BuildingUse::Other);
        assert!(!classify(&tall_box, None).is_pitched(), "девятиэтажка — не частный сектор");
        assert!(!classify(&plain(BuildingUse::Apartments), None).is_pitched());
        assert!(!classify(&building(square(), None, AreaKind::Kremlin, BuildingUse::Other), None).is_pitched());
    }
}
