//! Храмы: чем их кроют, во что красят и что над ними стоит.
//!
//! Храм сверху узнают не по цвету кровли, а по **силуэту над ней**, и у каждой
//! веры он свой ([`Faith`]):
//!
//! * **православный** — побелка, вальма зелёного или синего железа, луковичные
//!   главы на барабанах (одна у малого храма, пять у большого) и шатровая
//!   колокольня с западного конца, если храм вытянут «кораблём»;
//! * **западный** (католики, протестанты) — кирпич или камень, крутая
//!   двускатная кровля и башня со шпилем у западного фасада;
//! * **мечеть** — плоская кровля, полусферический купол и минареты по углам;
//! * **синагога** — камень под вальмой, крупная — с невысоким куполом;
//! * **восточный храм** — красные стены под тёмной вальмой.
//!
//! Форма кровли назначается здесь ([`roof_form`]) и строится в `roofs.rs`;
//! главы, башни и минареты — **венец** храма ([`Crown`]) — раскладываются по
//! плану ([`crowns`]) и рисуются поверх кровли ([`push_crowns`]) в том же
//! меше, что и сам дом, — но после всех домов слоя.
//!
//! Раскладка — по **минимальному описанному прямоугольнику** плана: где алтарь,
//! а где притвор, данные не говорят. Храм, собранный из частей (барабаны,
//! колокольня, пристройки), собирается в [`Sanctuary`], а венец каждой части
//! раскладывается по её собственному плану. Восток берётся за «алтарный»
//! конец длинной оси, запад — за вход и колокольню: так ориентировано
//! большинство храмов всех трёх христианских ветвей.
//!
//! Глава рисуется **стопкой ломтиков**, от основания к маковке: каждый — веер
//! с цветом по нормали поверхности в этой точке, поднятый по крену дома на
//! свою высоту. Верхний ломтик кроет нижний, и от нижнего остаётся ровно
//! ближний к камере серп — то, что и видно у настоящего купола. В плоских
//! режимах крена нет, ломтики ложатся концентрически, и сверху глава
//! читается кольцами света.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::color::Mix;
use bevy::prelude::*;

use super::heights::height_or_default;
use super::layers::wall_colors;
use super::material::{RoofKind, building_seed, look_seed};
use super::roofs::{LandmarkRoof, landmark_rise};
use super::{Lean, shade_by_light};
use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{BuildingUse, Faith, PolyArea, Sacred, SacredForm};
use crate::map::{shadow_length_scale, sun_light};

// ─── палитры ────────────────────────────────────────────────────────────────

/// Православный храм: побелка, охра, голубая и розовая штукатурка.
pub(super) const ORTHODOX_WALLS: [Color; 6] = [
    Color::srgb(0.93, 0.91, 0.86),
    Color::srgb(0.92, 0.86, 0.70),
    Color::srgb(0.90, 0.80, 0.56),
    Color::srgb(0.74, 0.81, 0.86),
    Color::srgb(0.88, 0.74, 0.70),
    Color::srgb(0.90, 0.89, 0.85),
];
/// Кровля православного храма — крашеное железо: зелёное, как его рисуют на
/// картах, бирюза, синее, серое и оцинковка.
pub const ORTHODOX_ROOFS: [Color; 5] = [
    Color::srgb(0.32, 0.52, 0.42),
    Color::srgb(0.30, 0.48, 0.50),
    Color::srgb(0.34, 0.42, 0.54),
    Color::srgb(0.42, 0.43, 0.44),
    Color::srgb(0.62, 0.64, 0.65),
];
/// Главы: золото — у каждой третьей, дальше зелень, синева, серебро, бирюза.
const ORTHODOX_DOMES: [Color; 6] = [
    Color::srgb(0.86, 0.66, 0.24),
    Color::srgb(0.82, 0.62, 0.28),
    Color::srgb(0.24, 0.48, 0.34),
    Color::srgb(0.22, 0.34, 0.60),
    Color::srgb(0.74, 0.76, 0.78),
    Color::srgb(0.28, 0.54, 0.52),
];
/// Костёл и кирха: красный и тёмный кирпич, серый камень, песчаник.
const WESTERN_WALLS: [Color; 5] = [
    Color::srgb(0.64, 0.38, 0.30),
    Color::srgb(0.54, 0.32, 0.27),
    Color::srgb(0.68, 0.66, 0.62),
    Color::srgb(0.78, 0.70, 0.56),
    Color::srgb(0.84, 0.82, 0.78),
];
/// Сланец, тёмная черепица и позеленевшая медь.
const WESTERN_ROOFS: [Color; 4] = [
    Color::srgb(0.30, 0.32, 0.35),
    Color::srgb(0.50, 0.25, 0.21),
    Color::srgb(0.40, 0.57, 0.51),
    Color::srgb(0.36, 0.37, 0.38),
];
const WESTERN_DOMES: [Color; 2] = [Color::srgb(0.40, 0.57, 0.51), Color::srgb(0.58, 0.60, 0.62)];
/// Мечеть: белая и песочная.
const MUSLIM_WALLS: [Color; 3] = [
    Color::srgb(0.93, 0.92, 0.89),
    Color::srgb(0.86, 0.78, 0.62),
    Color::srgb(0.90, 0.86, 0.77),
];
const MUSLIM_ROOFS: [Color; 2] = [Color::srgb(0.78, 0.76, 0.72), Color::srgb(0.72, 0.71, 0.68)];
const MUSLIM_DOMES: [Color; 4] = [
    Color::srgb(0.24, 0.56, 0.56),
    Color::srgb(0.22, 0.50, 0.36),
    Color::srgb(0.60, 0.62, 0.64),
    Color::srgb(0.84, 0.68, 0.30),
];
/// Синагога: песчаник, кирпич, серый камень.
const JEWISH_WALLS: [Color; 3] = [
    Color::srgb(0.80, 0.72, 0.58),
    Color::srgb(0.64, 0.40, 0.32),
    Color::srgb(0.70, 0.68, 0.64),
];
const JEWISH_ROOFS: [Color; 2] = [Color::srgb(0.44, 0.46, 0.48), Color::srgb(0.38, 0.52, 0.46)];
const JEWISH_DOMES: [Color; 2] = [Color::srgb(0.40, 0.56, 0.50), Color::srgb(0.62, 0.64, 0.66)];
/// Восточный храм: киноварь, охра, белёный камень.
const EASTERN_WALLS: [Color; 3] = [
    Color::srgb(0.66, 0.24, 0.20),
    Color::srgb(0.80, 0.62, 0.36),
    Color::srgb(0.88, 0.86, 0.82),
];
const EASTERN_ROOFS: [Color; 3] = [
    Color::srgb(0.30, 0.31, 0.32),
    Color::srgb(0.60, 0.46, 0.26),
    Color::srgb(0.34, 0.46, 0.36),
];
const EASTERN_DOMES: [Color; 2] = [Color::srgb(0.86, 0.68, 0.28), Color::srgb(0.90, 0.89, 0.86)];

/// Облицовка храма этой веры.
pub(super) fn wall_palette(faith: Faith) -> &'static [Color] {
    match faith {
        Faith::Orthodox => &ORTHODOX_WALLS,
        Faith::Western | Faith::Unknown => &WESTERN_WALLS,
        Faith::Muslim => &MUSLIM_WALLS,
        Faith::Jewish => &JEWISH_WALLS,
        Faith::Eastern => &EASTERN_WALLS,
    }
}

/// Кровля храма этой веры.
pub(super) fn roof_palette(faith: Faith) -> &'static [Color] {
    match faith {
        Faith::Orthodox => &ORTHODOX_ROOFS,
        Faith::Western | Faith::Unknown => &WESTERN_ROOFS,
        Faith::Muslim => &MUSLIM_ROOFS,
        Faith::Jewish => &JEWISH_ROOFS,
        Faith::Eastern => &EASTERN_ROOFS,
    }
}

fn dome_palette(faith: Faith) -> &'static [Color] {
    match faith {
        Faith::Orthodox => &ORTHODOX_DOMES,
        Faith::Western | Faith::Unknown => &WESTERN_DOMES,
        Faith::Muslim => &MUSLIM_DOMES,
        Faith::Jewish => &JEWISH_DOMES,
        Faith::Eastern => &EASTERN_DOMES,
    }
}

/// Чем крыт храм: железо у православных и синагог, черепица и сланец у
/// западных и восточных, у мечети — светлая плоская засыпка.
pub(super) fn roof_kind(sacred: Sacred) -> RoofKind {
    match sacred.faith {
        Faith::Orthodox | Faith::Jewish => RoofKind::Seam,
        Faith::Western | Faith::Unknown | Faith::Eastern => RoofKind::Tile,
        Faith::Muslim => RoofKind::Gravel,
    }
}

// ─── форма кровли ───────────────────────────────────────────────────────────

/// Часть храма не шире этого — барабан под главой, и кровли у неё не видно:
/// её закрывает сама глава, м. Шире — мапер пометил главой весь четверик, и
/// рисуется он храмом с главами.
const DOME_PART_MAX_SIDE: f32 = 14.0;

/// Форма кровли храма.
pub(super) fn roof_form(sacred: Sacred, building: &PolyArea) -> LandmarkRoof {
    if is_drum(sacred, building) {
        return LandmarkRoof::Flat;
    }
    match (sacred.faith, sacred.form) {
        (Faith::Orthodox, SacredForm::Tower) => LandmarkRoof::Tent { rise: 1.1 },
        (Faith::Western | Faith::Unknown, SacredForm::Tower) => LandmarkRoof::Tent { rise: 2.6 },
        (Faith::Muslim, SacredForm::Tower) => LandmarkRoof::Flat,
        (Faith::Jewish | Faith::Eastern, SacredForm::Tower) => LandmarkRoof::Tent { rise: 0.9 },
        (Faith::Orthodox | Faith::Jewish | Faith::Eastern, _) => LandmarkRoof::Hip,
        (Faith::Western | Faith::Unknown, _) => LandmarkRoof::SteepGable,
        (Faith::Muslim, _) => LandmarkRoof::Flat,
    }
}

/// Барабан под главой: часть с `roof:shape=onion|dome`, узкая настолько, что
/// сама она и есть барабан.
fn is_drum(sacred: Sacred, building: &PolyArea) -> bool {
    sacred.form == SacredForm::Dome
        && Plan::of(building).is_some_and(|plan| plan.width <= DOME_PART_MAX_SIDE)
}

// ─── венец ──────────────────────────────────────────────────────────────────

/// Что стоит над кровлей храма. Позиции — в **настоящих** координатах плана;
/// `base` — метров над карнизом, с которых элемент начинается (глава на
/// коньке вальмы стоит на её подъёме, колокольня — прямо на карнизе).
#[derive(Clone, Copy, Debug)]
pub(super) enum Crown {
    /// Глава на барабане.
    Dome {
        at: Vec2,
        radius: f32,
        base: f32,
        /// Высота барабана, м; ноль — глава садится прямо на кровлю.
        drum: f32,
        profile: Profile,
        color: Srgba,
        drum_color: Srgba,
        /// Барабан приподнятой части ([`Sanctuary`]): его высота настоящая и
        /// не вытягивается.
        raised: bool,
    },
    /// Колокольня или западная башня: квадратный столп, шатёр или шпиль над
    /// ним, у православной — ещё и маленькая глава на вершине.
    Tower {
        at: Vec2,
        axis: Vec2,
        side: f32,
        base: f32,
        height: f32,
        spire: f32,
        wall: Srgba,
        roof: Srgba,
        cap: Option<Srgba>,
    },
    /// Минарет: ствол, балкон муэдзина, ствол потоньше и конус.
    Minaret {
        at: Vec2,
        radius: f32,
        base: f32,
        height: f32,
        wall: Srgba,
        cap: Srgba,
    },
}

/// Профиль главы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Profile {
    /// Луковица: пузо шире барабана и острая маковка.
    Onion,
    /// Полусфера мечети, синагоги, барочного собора.
    Hemisphere,
}

impl Profile {
    /// Высота главы в её радиусах.
    fn height(self) -> f32 {
        match self {
            Self::Onion => 2.1,
            Self::Hemisphere => 1.0,
        }
    }

    /// Ломтик на доле высоты `t`: радиус в долях радиуса главы, высота в тех же
    /// долях и нормаль поверхности как `(cos, sin)` её угла к горизонту.
    fn sample(self, t: f32) -> (f32, f32, f32, f32) {
        match self {
            Self::Onion => {
                let radius = onion_radius(t);
                let height = self.height();
                // наклон образующей — численно: луковица склеена из двух дуг
                let step = 1e-3;
                let slope = (onion_radius((t + step).min(1.0)) - onion_radius((t - step).max(0.0)))
                    / ((t + step).min(1.0) - (t - step).max(0.0))
                    / height;
                let norm = (1.0 + slope * slope).sqrt();
                (radius, t * height, 1.0 / norm, -slope / norm)
            }
            Self::Hemisphere => {
                let angle = t * FRAC_PI_2;
                (angle.cos(), angle.sin(), angle.cos(), angle.sin())
            }
        }
    }
}

/// Доля высоты, на которой луковица шире всего.
const ONION_BELLY: f32 = 0.32;
/// Радиус основания луковицы в долях пуза — на нём она сидит на барабане.
const ONION_NECK: f32 = 0.72;

/// Образующая луковицы: от шейки до пуза четверть синусоиды, от пуза до
/// маковки — эрмитов сбег `1 − 3u² + 2u³`: выпуклый у пуза и вогнутый у
/// острия, то есть ровно та S-образная линия, которой луковица и отличается от
/// яйца. Косинус в степени, стоявший тут первым, давал сбег выпуклым до самой
/// маковки, и глава читалась яйцом.
fn onion_radius(t: f32) -> f32 {
    if t <= ONION_BELLY {
        ONION_NECK + (1.0 - ONION_NECK) * (t / ONION_BELLY * FRAC_PI_2).sin()
    } else {
        let u = ((t - ONION_BELLY) / (1.0 - ONION_BELLY)).min(1.0);
        1.0 - 3.0 * u * u + 2.0 * u * u * u
    }
}

/// Площадь, ниже которой православный храм — часовня с одной главкой, м².
const CHAPEL_AREA_MAX: f32 = 120.0;
/// Во сколько раз длина плана должна превышать ширину, чтобы храм читался
/// «кораблём» — с трапезной и колокольней по оси, и не короче этого, м.
const SHIP_RATIO_MIN: f32 = 1.6;
const SHIP_LENGTH_MIN: f32 = 22.0;
/// Длина, с которой у западного храма есть башня у входа, м.
const WESTERN_TOWER_LENGTH_MIN: f32 = 20.0;
/// Сколько православных храмов из десяти, кому хватает места, пятиглавы.
const FIVE_DOMES_SHARE: u32 = 6;
/// С какой ширины ядра храму хватает места на пять глав, м.
const FIVE_DOMES_MIN_SIDE: f32 = 14.0;
/// Площади, с которых у мечети второй и четыре минарета, м².
const MINARETS_TWO_AREA: f32 = 500.0;
const MINARETS_FOUR_AREA: f32 = 2000.0;
/// С какой площади у синагоги купол, м².
const SYNAGOGUE_DOME_AREA: f32 = 300.0;

/// План храма в раме его длинной оси.
#[derive(Clone, Copy)]
struct Plan {
    center: Vec2,
    /// Длинная ось, повёрнутая на восток — к алтарю.
    axis: Vec2,
    perp: Vec2,
    length: f32,
    width: f32,
    area: f32,
}

impl Plan {
    fn of(building: &PolyArea) -> Option<Self> {
        let rect = min_area_rect(&building.outer)?;
        let (first, second) = (rect[1] - rect[0], rect[2] - rect[1]);
        let (long, short) = match first.length() >= second.length() {
            true => (first, second),
            false => (second, first),
        };
        let mut axis = long.try_normalize()?;
        if axis.x < 0.0 || (axis.x == 0.0 && axis.y < 0.0) {
            axis = -axis;
        }
        Some(Self {
            center: (rect[0] + rect[2]) * 0.5,
            axis,
            perp: Vec2::new(-axis.y, axis.x),
            length: long.length(),
            width: short.length(),
            area: signed_ring_area(&building.outer).abs(),
        })
    }

    /// Углы плана со вдвигом `inset` — где встать минарету.
    fn corners(&self, inset: f32) -> [Vec2; 4] {
        let (half_l, half_w) = (
            (self.length / 2.0 - inset).max(0.0),
            (self.width / 2.0 - inset).max(0.0),
        );
        [
            self.center - self.axis * half_l - self.perp * half_w,
            self.center + self.axis * half_l + self.perp * half_w,
            self.center + self.axis * half_l - self.perp * half_w,
            self.center - self.axis * half_l + self.perp * half_w,
        ]
    }
}

/// Храмы города в сборе: какие части стоят на крыше своего храма и у каких
/// храмов главы размечены частями.
///
/// Нужно это потому, что часть храма нельзя нарисовать по одному её контуру. У
/// Успенского собора Тульского кремля барабаны глав — отдельные контуры с
/// `min_height=20`: они стоят на крыше собора, а коробкой от земли выходили
/// колоннами с окнами, проросшими сквозь его стены. И сам собор, у которого
/// главы уже есть частями, ставил поверх свои — лишний пучок глав.
pub(super) struct Sanctuary {
    /// Приподнятые части — индекс в списке домов и высота, с которой часть
    /// начинается, м. Коробки у них нет: рисуются барабан и глава.
    raised: HashMap<usize, f32>,
    /// Храмы (по посеву [`Sacred::complex`]), главы которых размечены частями:
    /// центральную главу такому храму от себя ставить незачем.
    domed: HashSet<u32>,
}

impl Sanctuary {
    pub(super) fn of(buildings: &[PolyArea]) -> Self {
        // хозяин храма — тот, от чьей первой вершины взят посев храма
        let hosts: HashMap<u32, usize> = buildings
            .iter()
            .enumerate()
            .filter_map(|(index, building)| match building.building_use {
                BuildingUse::Church(sacred)
                    if sacred.complex != 0 && sacred.complex == building_seed(building) =>
                {
                    Some((sacred.complex, index))
                }
                _ => None,
            })
            .collect();
        let mut raised = HashMap::new();
        let mut domed = HashSet::new();
        for (index, building) in buildings.iter().enumerate() {
            let BuildingUse::Church(sacred) = building.building_use else {
                continue;
            };
            if sacred.form != SacredForm::Dome {
                continue;
            }
            let host = hosts
                .get(&sacred.complex)
                .copied()
                .filter(|&host| host != index);
            let floor = match (sacred.floor(), host) {
                (floor, _) if floor > 0.0 => floor,
                // барабан без высоты начала, но на чужом храме — стоит на его крыше
                (_, Some(host)) if is_drum(sacred, building) => height_or_default(&buildings[host]),
                _ => continue,
            };
            // высота начала не выше самой части: иначе рисовать нечего
            let floor = floor.min(height_or_default(building) * RAISED_FLOOR_SHARE_MAX);
            raised.insert(index, floor);
            if host.is_some() {
                domed.insert(sacred.complex);
            }
        }
        Self { raised, domed }
    }

    /// С какой высоты начинается приподнятая часть; `None` — дом стоит на земле
    /// и рисуется коробкой.
    pub(super) fn raised(&self, index: usize) -> Option<f32> {
        self.raised.get(&index).copied()
    }

    /// Венец дома с его посадкой: `lift` — подъём карниза этого дома в текущем
    /// режиме. У приподнятой части посадка нулевая, а высота начала уже в
    /// самом элементе, — она меряется от земли, а не от своего карниза.
    pub(super) fn crowns(
        &self,
        index: usize,
        building: &PolyArea,
        wall: Srgba,
        roof: Srgba,
        lift: Vec2,
    ) -> Vec<(Crown, Vec2)> {
        if let Some(floor) = self.raised(index) {
            return raised_drum(building, floor, wall)
                .into_iter()
                .map(|crown| (crown, Vec2::ZERO))
                .collect();
        }
        let own_domes = match building.building_use {
            BuildingUse::Church(sacred) => !self.domed.contains(&sacred.complex),
            _ => true,
        };
        crowns(building, wall, roof, own_domes)
            .into_iter()
            .map(|crown| (crown, lift))
            .collect()
    }

    /// Пятна венца на земле и верх каждого над землёй, м: по ним теневой слой
    /// дотягивает тень храма до маковки (`layers::ShadowSweeps`). Контуры
    /// выпуклые и против часовой.
    pub(super) fn shadow_casters(
        &self,
        index: usize,
        building: &PolyArea,
    ) -> Vec<(Vec<Vec2>, f32)> {
        if !matches!(building.building_use, BuildingUse::Church(_)) {
            return Vec::new();
        }
        let eave = match self.raised(index) {
            Some(_) => 0.0,
            None => height_or_default(building),
        };
        self.crowns(index, building, Srgba::WHITE, Srgba::WHITE, Vec2::ZERO)
            .iter()
            .map(|(crown, _)| {
                let outline = match *crown {
                    Crown::Dome { at, radius, .. } => disc(at, radius, DOME_SIDES),
                    Crown::Tower { at, axis, side, .. } => square(at, axis, side).to_vec(),
                    Crown::Minaret { at, radius, .. } => disc(at, radius, SHAFT_SIDES),
                };
                (outline, eave + top(crown))
            })
            .collect()
    }
}

/// Высота начала приподнятой части не выше этой доли её собственной высоты:
/// `min_height` выше `height` — ошибка разметки, и барабан вышел бы нулевым.
const RAISED_FLOOR_SHARE_MAX: f32 = 0.8;

/// Приподнятая часть: барабан от высоты начала до верха части и глава на нём.
/// Верх части в OSM — это маковка (`height` вместе с `roof:height`), поэтому
/// барабан кончается там, где начинается глава.
fn raised_drum(building: &PolyArea, floor: f32, wall: Srgba) -> Option<Crown> {
    let BuildingUse::Church(sacred) = building.building_use else {
        return None;
    };
    let plan = Plan::of(building)?;
    let (profile, radius) = drum_cupola(sacred.faith, &plan);
    let drum = (height_or_default(building) - floor - radius * profile.height()).max(radius * 0.6);
    Some(Crown::Dome {
        at: plan.center,
        radius,
        base: floor,
        drum,
        profile,
        color: dome_color(building, sacred.faith),
        drum_color: wall,
        raised: true,
    })
}

/// Глава барабана — части, узкой настолько, что она сама и есть барабан:
/// профиль по вере и радиус по ширине части.
fn drum_cupola(faith: Faith, plan: &Plan) -> (Profile, f32) {
    let profile = match faith {
        Faith::Orthodox => Profile::Onion,
        _ => Profile::Hemisphere,
    };
    (profile, (plan.width * 0.45).clamp(1.2, 6.0))
}

/// Цвет глав — по посеву **храма** ([`look_seed`]): у частей одного собора главы
/// одного цвета.
fn dome_color(building: &PolyArea, faith: Faith) -> Srgba {
    let domes = dome_palette(faith);
    domes[(look_seed(building) >> 18) as usize % domes.len()].to_srgba()
}

/// Венец храма: `wall` и `roof` — цвета, которые дому уже выбрали стена и
/// кровля (барабан красится стеной, шатёр колокольни — кровлей). Не храм и
/// пристройка — пусто. `own_domes` — ставить ли храму центральную главу от
/// себя: `false`, когда главы у него размечены частями ([`Sanctuary`]).
pub(super) fn crowns(building: &PolyArea, wall: Srgba, roof: Srgba, own_domes: bool) -> Vec<Crown> {
    let BuildingUse::Church(sacred) = building.building_use else {
        return Vec::new();
    };
    if sacred.form == SacredForm::Annex {
        return Vec::new();
    }
    let Some(plan) = Plan::of(building) else {
        return Vec::new();
    };
    let seed = building_seed(building);
    let dome_color = dome_color(building, sacred.faith);
    // подъём кровли над карнизом: глава на вальме стоит на её площадке
    let rise = landmark_rise(building);
    let mut out = Vec::new();

    let dome = |at: Vec2, radius: f32, base: f32, drum: f32, profile: Profile| Crown::Dome {
        at,
        radius,
        base,
        drum,
        profile,
        color: dome_color,
        drum_color: wall,
        raised: false,
    };

    if is_drum(sacred, building) {
        let (profile, radius) = drum_cupola(sacred.faith, &plan);
        out.push(dome(plan.center, radius, 0.0, 0.0, profile));
        return out;
    }

    match (sacred.faith, sacred.form) {
        (Faith::Orthodox, SacredForm::Tower) => {
            let side = plan.area.sqrt();
            let radius = (side * 0.14).clamp(0.7, 2.2);
            out.push(dome(
                plan.center,
                radius,
                rise,
                radius * 0.8,
                Profile::Onion,
            ));
        }
        (Faith::Muslim, SacredForm::Tower) => {
            let side = plan.area.sqrt();
            out.push(Crown::Minaret {
                at: plan.center,
                radius: (side * 0.3).clamp(1.0, 2.5),
                base: 0.0,
                height: 4.0,
                wall: wall.mix(&Srgba::WHITE, 0.1),
                cap: dome_color,
            });
        }
        (_, SacredForm::Tower) => {}
        (Faith::Orthodox, _) => orthodox_nave(
            &mut out, &plan, seed, own_domes, rise, wall, roof, dome_color,
        ),
        (Faith::Western | Faith::Unknown, _) => {
            if plan.length >= WESTERN_TOWER_LENGTH_MIN {
                let side = (plan.width * 0.5).clamp(4.0, 10.0);
                out.push(Crown::Tower {
                    at: plan.center - plan.axis * (plan.length / 2.0 - side / 2.0),
                    axis: plan.axis,
                    side,
                    base: 0.0,
                    height: (plan.width * 0.8 + 6.0).clamp(10.0, 30.0),
                    spire: (side * 2.4).clamp(8.0, 32.0),
                    wall,
                    roof,
                    cap: None,
                });
            }
        }
        (Faith::Muslim, _) => {
            let radius = (plan.width.min(plan.length) * 0.3).clamp(2.0, 14.0);
            if own_domes {
                out.push(dome(
                    plan.center,
                    radius,
                    0.0,
                    radius * 0.25,
                    Profile::Hemisphere,
                ));
            }
            let count = match plan.area {
                area if area >= MINARETS_FOUR_AREA => 4,
                area if area >= MINARETS_TWO_AREA => 2,
                _ => 1,
            };
            let minaret = (plan.width * 0.05).clamp(1.0, 2.2);
            for at in plan.corners(minaret + 1.2).into_iter().take(count) {
                out.push(Crown::Minaret {
                    at,
                    radius: minaret,
                    base: 0.0,
                    height: (plan.width.max(10.0) * 1.2).clamp(12.0, 40.0),
                    wall: wall.mix(&Srgba::WHITE, 0.1),
                    cap: dome_color,
                });
            }
        }
        (Faith::Jewish, _) => {
            if own_domes && plan.area >= SYNAGOGUE_DOME_AREA {
                let radius = (plan.width * 0.16).clamp(2.0, 5.0);
                out.push(dome(
                    plan.center,
                    radius,
                    rise,
                    radius * 0.6,
                    Profile::Hemisphere,
                ));
            }
        }
        (Faith::Eastern, _) => {}
    }
    out
}

/// Православный храм: главы и колокольня. Без `own_domes` — одна колокольня:
/// главы у храма размечены частями.
#[allow(clippy::too_many_arguments)]
fn orthodox_nave(
    out: &mut Vec<Crown>,
    plan: &Plan,
    seed: u32,
    own_domes: bool,
    rise: f32,
    wall: Srgba,
    roof: Srgba,
    dome_color: Srgba,
) {
    let dome = |at: Vec2, radius: f32, drum: f32| Crown::Dome {
        at,
        radius,
        base: rise,
        drum,
        profile: Profile::Onion,
        color: dome_color,
        drum_color: wall,
        raised: false,
    };
    if plan.area < CHAPEL_AREA_MAX {
        let radius = (plan.width.min(plan.length) * 0.3).clamp(1.0, 3.0);
        out.push(dome(plan.center, radius, radius * 0.9));
        return;
    }
    // «корабль»: трапезная и колокольня по оси, главы — над восточным ядром
    let ship = plan.length >= plan.width * SHIP_RATIO_MIN && plan.length >= SHIP_LENGTH_MIN;
    let (core, core_length) = if ship {
        let side = (plan.width * 0.6).clamp(4.0, 9.0);
        out.push(Crown::Tower {
            at: plan.center - plan.axis * (plan.length / 2.0 - side / 2.0),
            axis: plan.axis,
            side,
            base: 0.0,
            height: (plan.width * 1.2).clamp(8.0, 28.0),
            spire: side * 1.5,
            wall,
            roof,
            cap: Some(dome_color),
        });
        // ядро — восточная часть за колокольней и трапезной
        let core_length = (plan.length - side) * 0.6;
        (
            plan.center + plan.axis * (plan.length / 2.0 - core_length / 2.0),
            core_length,
        )
    } else {
        (plan.center, plan.length)
    };
    let side = core_length.min(plan.width);
    let radius = (side * 0.2).clamp(1.6, 6.0);
    // главы, размеченные частями, заменяют все свои: малые главы вокруг них
    // встали бы вперемешку с настоящими и слиплись бы с ними парами
    let five = own_domes && side >= FIVE_DOMES_MIN_SIDE && (seed >> 11) % 10 < FIVE_DOMES_SHARE;
    if five {
        let offset = side * 0.28;
        for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let at = core + plan.axis * (u * offset) + plan.perp * (v * offset);
            out.push(dome(at, radius * 0.5, radius * 0.9));
        }
    }
    if own_domes {
        out.push(dome(core, radius, radius * 1.1));
    }
}

/// Верх элемента над карнизом, м, — докуда он отбрасывает тень.
fn top(crown: &Crown) -> f32 {
    match *crown {
        Crown::Dome {
            radius,
            base,
            drum,
            profile,
            ..
        } => base + drum + radius * profile.height(),
        Crown::Tower {
            base,
            height,
            spire,
            cap,
            side,
            ..
        } => {
            let cap = cap.map_or(0.0, |_| {
                cap_radius(side) * (CAP_DRUM + Profile::Onion.height())
            });
            base + height + spire + cap
        }
        Crown::Minaret {
            radius,
            base,
            height,
            ..
        } => base + height + radius * CONE_RISE,
    }
}

// ─── отрисовка ──────────────────────────────────────────────────────────────

/// Во сколько раз луковица и её барабан вытянуты вверх против остального
/// дома. Подъём 2.5D сжимает высоту втрое (`EXTRUDE_SCALE`), и честная
/// луковица на экране выходила шаром: силуэт главы держится ровно на её
/// вытянутости. Полусфера вытянута слабее — на полной вытяжке купол мечети
/// становился яйцом. Тень главы меряется настоящими метрами ([`top`]) —
/// вытягивается рисунок, а не храм.
const ONION_STRETCH: f32 = 2.6;
const HEMISPHERE_STRETCH: f32 = 1.4;

impl Profile {
    fn stretch(self) -> f32 {
        match self {
            Self::Onion => ONION_STRETCH,
            Self::Hemisphere => HEMISPHERE_STRETCH,
        }
    }
}
/// Граней у ломтика главы и у барабана.
const DOME_SIDES: usize = 20;
/// Ломтиков у главы.
const DOME_SLICES: usize = 16;
/// Граней у ствола минарета.
const SHAFT_SIDES: usize = 12;
/// Подъём конуса минарета в радиусах ствола.
const CONE_RISE: f32 = 3.0;
/// Сколько от главы остаётся в тени и сколько добавляет свет: тон ломтика —
/// `AMBIENT + DIFFUSE × ламберт`.
const AMBIENT: f32 = 0.52;
const DIFFUSE: f32 = 0.62;
/// Блик металла главы: доля смеси к белому на ламберте в единицу.
const SPECULAR: f32 = 0.35;
/// Тон стен барабана и ствола по свету — как у цилиндров промзоны.
const SHAFT_LIT_MIX: f32 = 0.26;
const SHAFT_SHADED_MIX: f32 = 0.26;
/// Тёмный проём — окно барабана и арка звона.
const OPENING_COLOR: Color = Color::srgb(0.16, 0.15, 0.15);

/// Венцы поверх кровель: у каждого элемента своя посадка — подъём карниза его
/// дома (у приподнятой части ноль), `lean` — крен в 2.5D (`None` в плоских
/// режимах, где стен у венца не видно). Элементы кладутся от дальнего к
/// ближнему, как и дома.
///
/// Вызывающий кладёт сюда венцы **всех** домов слоя разом и **после** всех
/// домов, а не каждый дом свой: венец выше любой кровли вокруг, а храм в OSM —
/// это несколько перекрывающихся контуров, и пристройка, положенная после
/// собора, закрывала низ его глав — главы торчали из-за её стен.
pub(super) fn push_crowns(builder: &mut MeshBuilder, crowns: &[(Crown, Vec2)], lean: Option<Lean>) {
    if crowns.is_empty() {
        return;
    }
    builder.set_roof(None);
    let order = Lean::of();
    let mut sorted: Vec<&(Crown, Vec2)> = crowns.iter().collect();
    sorted.sort_by(|a, b| {
        order
            .depth(at_of(&b.0))
            .total_cmp(&order.depth(at_of(&a.0)))
    });
    let light = Light::now();
    for &(ref crown, eave) in sorted {
        match *crown {
            Crown::Dome {
                at,
                radius,
                base,
                drum,
                profile,
                color,
                drum_color,
                raised,
            } => {
                let seat = at + eave + up(lean, base);
                let drum_radius = match profile {
                    Profile::Onion => radius * ONION_NECK * 0.92,
                    Profile::Hemisphere => radius * 0.98,
                };
                // вытягивается короткий барабан под главой; у приподнятой части
                // барабан — настоящая высота из разметки, и вытянутый он унёс бы
                // главу выше колокольни
                let drum = match raised {
                    true => drum,
                    false => drum * profile.stretch(),
                };
                if drum > 0.0 {
                    push_shaft(builder, seat, drum_radius, drum, drum_color, lean, true);
                }
                push_dome(
                    builder,
                    seat + up(lean, drum),
                    radius,
                    profile,
                    color,
                    lean,
                    &light,
                );
            }
            Crown::Tower {
                at,
                axis,
                side,
                base,
                height,
                spire,
                wall,
                roof,
                cap,
            } => {
                let seat = at + eave + up(lean, base);
                push_tower(builder, seat, axis, side, height, spire, wall, roof, lean);
                if let Some(color) = cap {
                    let radius = cap_radius(side);
                    let apex = seat + up(lean, height + spire);
                    push_shaft(
                        builder,
                        apex,
                        radius * 0.6,
                        radius * CAP_DRUM,
                        color,
                        lean,
                        false,
                    );
                    push_dome(
                        builder,
                        apex + up(lean, radius * CAP_DRUM),
                        radius,
                        Profile::Onion,
                        color,
                        lean,
                        &light,
                    );
                }
            }
            Crown::Minaret {
                at,
                radius,
                base,
                height,
                wall,
                cap,
            } => {
                let seat = at + eave + up(lean, base);
                push_minaret(builder, seat, radius, height, wall, cap, lean);
            }
        }
    }
}

/// Высота барабанчика под маковкой колокольни в радиусах маковки.
const CAP_DRUM: f32 = 0.8;

/// Радиус маковки на вершине шатра колокольни.
fn cap_radius(side: f32) -> f32 {
    (side * 0.14).clamp(0.6, 2.0)
}

fn at_of(crown: &Crown) -> Vec2 {
    match *crown {
        Crown::Dome { at, .. } | Crown::Tower { at, .. } | Crown::Minaret { at, .. } => at,
    }
}

/// Сдвиг на `metres` настоящих метров вверх: в 2.5D — по крену, в плоских
/// режимах — никакого.
fn up(lean: Option<Lean>, metres: f32) -> Vec2 {
    lean.map_or(Vec2::ZERO, |lean| lean.ridge(metres))
}

/// Солнце в трёх измерениях: направление в плане и высота над горизонтом.
struct Light {
    plan: Vec2,
    cos_elevation: f32,
    sin_elevation: f32,
}

impl Light {
    fn now() -> Self {
        let cot = shadow_length_scale();
        let norm = (1.0 + cot * cot).sqrt();
        Self {
            plan: sun_light(),
            cos_elevation: cot / norm,
            sin_elevation: 1.0 / norm,
        }
    }

    /// Тон металла с нормалью `(outward · cos, sin)`.
    fn shade(&self, base: Srgba, outward: Vec2, cos: f32, sin: f32) -> LinearRgba {
        let lambert =
            (cos * self.cos_elevation * outward.dot(self.plan) + sin * self.sin_elevation).max(0.0);
        let k = AMBIENT + DIFFUSE * lambert;
        let toned = Srgba::new(
            (base.red * k).min(1.0),
            (base.green * k).min(1.0),
            (base.blue * k).min(1.0),
            1.0,
        );
        toned.mix(&Srgba::WHITE, lambert.powi(12) * SPECULAR).into()
    }
}

/// Глава стопкой ломтиков от `seat` вверх.
fn push_dome(
    builder: &mut MeshBuilder,
    seat: Vec2,
    radius: f32,
    profile: Profile,
    color: Srgba,
    lean: Option<Lean>,
    light: &Light,
) {
    let step = TAU / DOME_SIDES as f32;
    let directions: Vec<Vec2> = (0..DOME_SIDES)
        .map(|side| Vec2::from_angle(side as f32 * step))
        .collect();
    for slice in 0..DOME_SLICES {
        let (r, h, cos, sin) = profile.sample(slice as f32 / DOME_SLICES as f32);
        let r = r * radius;
        if r < 0.02 {
            continue;
        }
        let center = seat + up(lean, h * radius * profile.stretch());
        let rim: Vec<(Vec2, LinearRgba)> = directions
            .iter()
            .map(|&direction| {
                (
                    center + direction * r,
                    light.shade(color, direction, cos, sin),
                )
            })
            .collect();
        builder.push_fan_gradient(center, light.shade(color, light.plan, 0.0, 1.0), &rim);
    }
    // маковка с крестом — тонкая спица над главой; в плоских режимах её не видно
    if let Some(lean) = lean {
        let tip = seat + lean.ridge(radius * profile.height() * profile.stretch());
        let across = Vec2::new(-lean.dir().y, lean.dir().x) * (radius * 0.05).max(0.06);
        let top = tip + lean.ridge(radius * 0.7 * ONION_STRETCH);
        builder.push_quad(
            [tip - across, tip + across, top + across, top - across],
            color.mix(&Srgba::BLACK, 0.15).into(),
        );
    }
}

/// Цилиндр от `seat` на `height` метров: видимая половина граней с тоном по
/// свету, крышка сверху. `slits` — узкие окна барабана.
fn push_shaft(
    builder: &mut MeshBuilder,
    seat: Vec2,
    radius: f32,
    height: f32,
    color: Srgba,
    lean: Option<Lean>,
    slits: bool,
) {
    let rise = up(lean, height);
    if let Some(toward) = rise.try_normalize() {
        let step = TAU / DOME_SIDES as f32;
        let opening: LinearRgba = OPENING_COLOR.into();
        for side in 0..DOME_SIDES {
            let (from, to) = (
                Vec2::from_angle(side as f32 * step),
                Vec2::from_angle((side + 1) as f32 * step),
            );
            let outward = (from + to).normalize_or(from);
            if outward.dot(toward) > 0.0 {
                continue;
            }
            let (left, right) = (seat + from * radius, seat + to * radius);
            builder.push_quad(
                [left, right, right + rise, left + rise],
                shade_by_light(color, outward, SHAFT_LIT_MIX, SHAFT_SHADED_MIX).into(),
            );
            if slits && side % 2 == 0 && height >= 1.5 {
                let (a, b) = (left.lerp(right, 0.3), left.lerp(right, 0.7));
                let (low, high) = (rise * 0.3, rise * 0.75);
                builder.push_quad([a + low, b + low, b + high, a + high], opening);
            }
        }
    }
    builder.push_convex(
        &disc(seat + rise, radius, DOME_SIDES),
        color.mix(&Srgba::WHITE, 0.08).into(),
    );
}

/// Столп колокольни с шатром: видимые стены с аркой звона, потом грани шатра.
#[allow(clippy::too_many_arguments)]
fn push_tower(
    builder: &mut MeshBuilder,
    seat: Vec2,
    axis: Vec2,
    side: f32,
    height: f32,
    spire: f32,
    wall: Srgba,
    roof: Srgba,
    lean: Option<Lean>,
) {
    let base = square(seat, axis, side);
    let rise = up(lean, height);
    if let Some(lean) = lean {
        let opening: LinearRgba = OPENING_COLOR.into();
        for index in 0..4 {
            let (a, b) = (base[index], base[(index + 1) % 4]);
            let edge = b - a;
            let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero();
            if outward.dot(lean.dir()) > 0.0 {
                continue;
            }
            let (bottom, top) = wall_colors(wall, a, b, lean.dir());
            builder.push_quad_gradient([a, b, b + rise, a + rise], [bottom, bottom, top, top]);
            // арка звона под шатром
            let (left, right) = (a.lerp(b, 0.32), a.lerp(b, 0.68));
            let (low, high) = (rise * 0.74, rise * 0.9);
            builder.push_quad(
                [left + low, right + low, right + high, left + high],
                opening,
            );
        }
    }
    let ring = base.map(|corner| corner + rise);
    push_cone(builder, &ring, seat + up(lean, height + spire), roof, lean);
}

/// Минарет: ствол, балкон, ствол потоньше, конус.
fn push_minaret(
    builder: &mut MeshBuilder,
    seat: Vec2,
    radius: f32,
    height: f32,
    wall: Srgba,
    cap: Srgba,
    lean: Option<Lean>,
) {
    let lower = height * 0.8;
    push_shaft(builder, seat, radius, lower, wall, lean, false);
    let balcony = seat + up(lean, lower);
    builder.push_convex(
        &disc(balcony, radius * 1.45, SHAFT_SIDES),
        wall.mix(&Srgba::BLACK, 0.12).into(),
    );
    let upper = radius * 0.78;
    push_shaft(builder, balcony, upper, height - lower, wall, lean, false);
    let top = seat + up(lean, height);
    push_cone(
        builder,
        &disc(top, upper * 1.05, SHAFT_SIDES),
        top + up(lean, radius * CONE_RISE),
        cap,
        lean,
    );
}

/// Конус или шатёр: грани от кольца к вершине, отвёрнутые от камеры первыми.
fn push_cone(
    builder: &mut MeshBuilder,
    ring: &[Vec2],
    apex: Vec2,
    color: Srgba,
    lean: Option<Lean>,
) {
    let count = ring.len();
    let orientation = signed_ring_area(ring).signum();
    let toward_camera = lean.map_or(Vec2::ZERO, |lean| -lean.dir());
    let mut faces: Vec<(f32, [Vec2; 3], LinearRgba)> = (0..count)
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % count]);
            let edge = b - a;
            let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero() * orientation;
            let tone = shade_by_light(color, outward, 0.24, 0.26);
            (outward.dot(toward_camera), [a, b, apex], tone.into())
        })
        .collect();
    faces.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (_, face, tone) in faces {
        builder.push_triangle(face, tone);
    }
}

/// Круг правильным многоугольником, против часовой.
fn disc(at: Vec2, radius: f32, sides: usize) -> Vec<Vec2> {
    let step = TAU / sides as f32;
    (0..sides)
        .map(|side| at + Vec2::from_angle(side as f32 * step) * radius)
        .collect()
}

/// Квадрат со стороной `side` вокруг `at`, повёрнутый по `axis`, против часовой.
fn square(at: Vec2, axis: Vec2, side: f32) -> [Vec2; 4] {
    let perp = Vec2::new(-axis.y, axis.x);
    let (u, v) = (axis * (side / 2.0), perp * (side / 2.0));
    [at - u - v, at + u - v, at + u + v, at - u + v]
}
