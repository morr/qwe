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
use super::roofs::{LandmarkRoof, landmark_inset, landmark_rise};
use super::{Lean, shade_by_light};
use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::model::{point_in_area, signed_ring_area};
use crate::map::osm::{BuildingUse, Faith, PolyArea, Sacred, SacredForm, srgba_of};
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
/// Главы: золото у половины — как в Туле, где золотом крыты кремлёвский
/// собор, Всехсвятский и Николо-Зарецкая, — дальше зелень, синева и чернь
/// краснокирпичного Успенского. Серебро ушло: на снимке серая глава читается
/// оцинковкой сарая.
const ORTHODOX_DOMES: [Color; 6] = [
    Color::srgb(0.86, 0.66, 0.24),
    Color::srgb(0.82, 0.62, 0.28),
    Color::srgb(0.90, 0.72, 0.30),
    Color::srgb(0.24, 0.48, 0.34),
    Color::srgb(0.22, 0.34, 0.60),
    Color::srgb(0.20, 0.20, 0.22),
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

// ─── цвета из разметки ──────────────────────────────────────────────────────

/// Стена храма по `building:colour`, если он размечен; не храм — `None`, ему
/// цвет из тега пока не читается. Тула: `white` у Всехсвятского собора и его
/// колокольни, `red` у музея в здании кремлёвского собора (пристройка).
pub(super) fn tagged_wall(building: &PolyArea) -> Option<Srgba> {
    match building.building_use {
        BuildingUse::Church(_) => building.colours.wall.map(srgba_of),
        _ => None,
    }
}

/// Кровля храма по `roof:colour`. У части с `roof:shape=onion|dome`
/// ([`SacredForm::Dome`]) тег красит **главу**, а не кровлю — её там и нет
/// ([`tagged_dome`]); у колокольни — шатёр или шпиль вместе с маковкой.
pub(super) fn tagged_roof(building: &PolyArea) -> Option<Srgba> {
    match building.building_use {
        BuildingUse::Church(sacred) if sacred.form != SacredForm::Dome => {
            building.colours.roof.map(srgba_of)
        }
        _ => None,
    }
}

/// Глава по `roof:colour` — у части и у храма с `roof:shape=onion|dome`: там
/// «кровля» и есть глава. Кремлёвский собор: `#FFD700` на обоих барабанах —
/// золото, которое палитра по посеву красила серебром.
fn tagged_dome(building: &PolyArea) -> Option<Srgba> {
    match building.building_use {
        BuildingUse::Church(sacred) if sacred.form == SacredForm::Dome => {
            building.colours.roof.map(srgba_of)
        }
        _ => None,
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
        // коробки у неё нет — весь дом венец, и кровля тут ни к чему
        (Faith::Orthodox | Faith::Western | Faith::Unknown, SacredForm::Tower) => {
            LandmarkRoof::Flat
        }
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
    /// Колокольня или западная башня: столп **ярусами** — каждый следующий
    /// у́же и ниже предыдущего, между ними белый карниз, на верхнем — арки
    /// звона, — а над ним шатёр или шпиль, у православной ещё и маковка.
    /// Столп прямоугольный, а не квадратный, потому что чаще всего он садится
    /// на собственный выступ храма ([`Plan::west_piece`]), а тот квадратным не
    /// бывает.
    Tower {
        at: Vec2,
        axis: Vec2,
        /// Нижний ярус, вдоль `axis` × поперёк.
        size: Vec2,
        base: f32,
        /// Высота столпа — всех ярусов с карнизами, м.
        height: f32,
        /// Подъём шатра или шпиля над столпом, м.
        spire: f32,
        /// Ярусов, 1–3 ([`tier_count`]).
        tiers: u8,
        top: TowerTop,
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

/// Чем кончается столп колокольни.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TowerTop {
    /// Шатёр во весь верхний ярус — русская колокольня XVII века.
    Tent,
    /// Фонарь и тонкий шпиль — классицизм: кремлёвская и Всехсвятская
    /// колокольни Тулы, кирха и костёл.
    Spire,
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
/// Шаг, которым башня вдвигается от западного торца внутрь дома и сужается, м.
const TOWER_SEAT_STEP: f32 = 0.5;
/// Уже этого башня — уже не колокольня, а тумба: такой посадки лучше не быть, м.
const TOWER_SIDE_MIN: f32 = 4.0;
/// Насколько доля контура может не дотягивать до западного торца плана, м.
const TOWER_PIECE_REACH: f32 = 1.0;
/// Шире этого доля контура — уже не выступ, а сам храм, м.
const TOWER_PIECE_SIDE_MAX: f32 = 16.0;
/// Доля контура годится в основание, если заполняет свой прямоугольник настолько.
const TOWER_PIECE_FILL: f32 = 0.9;
/// Разлёт малых глав не ниже этой доли радиуса большой: ближе они с ней слипаются.
const DOME_SPREAD_MIN: f32 = 1.1;
/// Сколькими точками круг главы проверяется на посадку.
const DOME_SEAT_PROBES: usize = 8;
/// Зазор, с которым угол башни считается стоящим на доме, м: у прямоугольного
/// храма угол `min_area_rect` лежит ровно на стене, и без зазора башня съезжала
/// бы с торца на пустом месте. На столько же башня и свесится в худшем случае —
/// пять сантиметров при любом зуме меньше пикселя.
const TOWER_SEAT_SLACK: f32 = 0.05;
/// Сколько православных храмов из десяти, кому хватает места, пятиглавы.
const FIVE_DOMES_SHARE: u32 = 6;
/// Сколько православных колоколен из десяти — со шпилем, а не с шатром.
const SPIRE_SHARE_OF_10: u32 = 4;
/// Ярусы столпа: ярус на каждые столько узких сторон высоты, не больше трёх.
/// Кремлёвская колокольня Тулы (12 × 12 м, столп ~50 м) — три яруса, столп
/// корабля 6 × 22 м — два, часовенная башенка — один.
const TIER_ASPECT: f32 = 1.6;
const TIERS_MAX: u8 = 3;
/// Во сколько раз каждый следующий ярус у́же предыдущего.
const TIER_SHRINK: f32 = 0.8;
/// Столп над нижним ярусом не шире этого, м: контур отдельной колокольни в
/// OSM — это её нижний ярус вместе с папертями и боковыми палатами
/// (кремлёвская в Туле 28 × 24 м, Всехсвятская 24 × 24), а сам столп над ним —
/// десять-тринадцать метров. Без зажима вторым ярусом шёл тот же короб.
const SHAFT_SIDE_MAX: f32 = 12.0;
/// Нижний ярус шире столпа — не выше этого, м: он двухэтажный, а не куб в
/// двадцать четыре метра, каким выходил у Всехсвятской колокольни.
const BASE_TIER_MAX: f32 = 14.0;
/// Проёмы столпа метрами, а не долями стены: на 12-метровой стене доля выходила
/// чёрными воротами. Арка звона и окно глухого яруса — ширина × высота, м; и
/// не шире этой доли своей стены, чтобы на узком столпе не слиться в один проём.
const BELFRY_ARCH: Vec2 = Vec2::new(2.0, 5.0);
const TIER_WINDOW: Vec2 = Vec2::new(1.1, 2.4);
const OPENING_SHARE_MAX: f32 = 0.28;
/// Шаг арок звона вдоль стены, м, и потолок их числа на стене.
const BELFRY_PITCH: f32 = 4.5;
const BELFRY_ARCHES_MAX: usize = 3;
/// С какой длины стены на глухом ярусе два окна вместо одного, м.
const TWO_WINDOWS_FROM: f32 = 9.0;
/// Высоты ярусов снизу вверх, в долях друг друга: нижний самый высокий.
const TIER_SHARES: [f32; 3] = [1.0, 0.8, 0.65];
/// Карниз между ярусами: вылет за стену и высота, м.
const CORNICE_REACH: f32 = 0.35;
const CORNICE_HEIGHT: f32 = 0.5;
/// Насколько карниз светлее стены — он белёный.
const CORNICE_LIGHTEN: f32 = 0.3;
/// Доля высоты колокольни из OSM, которая столп; остальное — шатёр или шпиль.
/// У кремлёвской колокольни Тулы 70 м по тегу — это с шпилем.
const TOWER_PILLAR_SHARE: f32 = 0.72;
/// Фонарь под шпилем: радиус и высота в долях узкой стороны верхнего яруса.
const LANTERN_RADIUS: f32 = 0.28;
const LANTERN_HEIGHT: f32 = 0.45;
/// Основание шпиля в долях радиуса фонаря — он у́же фонаря, на его крыше.
const SPIRE_FOOT: f32 = 0.8;
/// Маковка на шпиле — шар, а не глава: доля от маковки шатра.
const SPIRE_BALL: f32 = 0.5;
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

    /// Посадка колокольни: центр, ось и размер её столпа.
    ///
    /// **Сначала — собственный западный выступ храма** ([`Plan::west_piece`]):
    /// столп ровно по нему значит, что стены башни — это стены храма,
    /// продолженные вверх, и стыка с кровлей не надо рисовать вовсе, его
    /// просто нет. Выступа нет или он не годится в основание — тогда общий
    /// поиск квадратом ([`Plan::west_tower`]), и стороной не шире `side`.
    fn tower_seat(&self, building: &PolyArea, side: f32) -> Option<(Vec2, Vec2, Vec2)> {
        self.west_piece(building).or_else(|| {
            self.west_tower(building, side)
                .map(|(at, width)| (at, self.axis, Vec2::splat(width)))
        })
    }

    /// Западный выступ храма прямоугольником: центр, его собственная ось и
    /// размер. Притвор, трапезная, само основание колокольни — то, чем храм
    /// выдаётся на запад; на нём колокольня и стоит.
    ///
    /// Контур режется хордой от каждой вогнутой вершины вдоль её собственной
    /// стены (`garages::cut_at` — приём крестовой кровли), берутся доли,
    /// упирающиеся в западный торец плана, и из них — **наименьшая**, которая
    /// заполняет свой `min_area_rect` на `TOWER_PIECE_FILL` и не мельче
    /// `TOWER_SIDE_MIN`. Наименьшая, а не любая: у Двенадцати Апостолов (Тула,
    /// way 42066388) одна хорда отрезает притвор 7.5 × 5.4 м, другая — притвор
    /// вместе с шеей, и колокольня стоит на первом.
    ///
    /// Ось берётся у самого выступа, а не у плана: у Свято-Никольского (way
    /// 234273451) стены западного придела повёрнуты к плану на пару градусов,
    /// и столп по оси плана вылезал бы из них сантиметрами — тем самым
    /// «небольшим зазором между крышей и башней».
    fn west_piece(&self, building: &PolyArea) -> Option<(Vec2, Vec2, Vec2)> {
        let ring = &building.outer;
        let count = ring.len();
        let winding = signed_ring_area(ring).signum();
        let mut best: Option<(f32, Vec2, Vec2, Vec2)> = None;
        for at in 0..count {
            let (prev, here, next) = (
                ring[(at + count - 1) % count],
                ring[at],
                ring[(at + 1) % count],
            );
            let (back, ahead) = (here - prev, next - here);
            if back.perp_dot(ahead) * winding >= 0.0 {
                continue;
            }
            for direction in [back, -ahead] {
                let Some(direction) = direction.try_normalize() else {
                    continue;
                };
                let Some((near, far)) = super::garages::cut_at(ring, at, direction) else {
                    continue;
                };
                for piece in [near, far] {
                    let Some(found) = self.piece_seat(building, &piece) else {
                        continue;
                    };
                    if best.is_none_or(|(area, ..)| found.0 < area) {
                        best = Some(found);
                    }
                }
            }
        }
        best.map(|(_, at, axis, size)| (at, axis, size))
    }

    /// Доля контура как основание колокольни: площадь (по ней выбирают
    /// наименьшую), центр, ось и размер. `None` — доля не у западного торца,
    /// мелка, велика, не прямоугольна или её прямоугольник вылез из дома.
    fn piece_seat(&self, building: &PolyArea, piece: &[Vec2]) -> Option<(f32, Vec2, Vec2, Vec2)> {
        let west = piece
            .iter()
            .map(|point| (*point - self.center).dot(self.axis))
            .fold(f32::MAX, f32::min);
        if west > -self.length / 2.0 + TOWER_PIECE_REACH {
            return None;
        }
        let rect = min_area_rect(piece)?;
        let (long, short) = (rect[1] - rect[0], rect[2] - rect[1]);
        let size = Vec2::new(long.length(), short.length());
        if size.min_element() < TOWER_SIDE_MIN || size.max_element() > TOWER_PIECE_SIDE_MAX {
            return None;
        }
        let area = signed_ring_area(piece).abs();
        if area < size.x * size.y * TOWER_PIECE_FILL {
            return None;
        }
        let (at, axis) = ((rect[0] + rect[2]) * 0.5, long.try_normalize()?);
        let stands = rect
            .iter()
            .all(|&corner| point_in_area(corner.move_towards(at, TOWER_SEAT_SLACK), building));
        stands.then_some((area, at, axis, size))
    }

    /// Посадка башни у западного торца: центр квадрата и его сторона, не шире
    /// `side`. **Башня должна стоять на доме всеми четырьмя углами** — правило
    /// кровельного оборудования (`clutter`), и оно не выполнялось само собой:
    /// `min_area_rect` описывает вместе с домом и крыльцо, и апсиду, так что у
    /// торца с притвором прямоугольник длиннее самого храма, и башня вровень с
    /// его концом висела над землёй (Тула, way 496756343 — 2.5 м в воздухе, и
    /// так у десяти из двадцати семи городских храмов).
    ///
    /// Поэтому квадрат вдвигается по оси внутрь шагами `TOWER_SEAT_STEP`, на
    /// каждом шаге сужается и на каждой ширине пробует сдвиг **поперёк** оси
    /// ([`nudges`]). Порядок перебора — правило раскладки: башня держится
    /// **западного торца**, так что выигрывает самый западный шаг, на нём —
    /// самая широкая башня, а на ней — наименьший сдвиг с середины торца.
    ///
    /// Каждая из трёх свобод отвечает за свой случай. Сужение оставляет
    /// западную грань на месте, так что от прямой стены оно не спасает, а от
    /// узкого притвора спасает. Поперечный сдвиг нужен потому, что притвор в
    /// данных редко стоит ровно по оси плана: у Двенадцати Апостолов (Тула, way
    /// 42066388) настоящее основание колокольни 7.5 × 5.4 м смещено с неё на
    /// 0.9 м, и башня, которой сдвинуться было нечем, уезжала с него на шею
    /// храма и вставала там впритык к её стене — стык с кровлей выходил кривым.
    fn west_tower(&self, building: &PolyArea, side: f32) -> Option<(Vec2, f32)> {
        let west = self.center - self.axis * (self.length / 2.0 - side / 2.0);
        let mut slide = 0.0;
        while slide <= self.length / 2.0 {
            let mut width = side;
            while width >= TOWER_SIDE_MIN {
                let along = west + self.axis * (slide - (side - width) / 2.0);
                let seat = nudges((self.width - width) / 2.0)
                    .map(|shift| along + self.perp * shift)
                    .find(|&at| {
                        rect(at, self.axis, Vec2::splat(width - 2.0 * TOWER_SEAT_SLACK))
                            .iter()
                            .all(|&corner| point_in_area(corner, building))
                    });
                if let Some(at) = seat {
                    return Some((at, width));
                }
                width -= TOWER_SEAT_STEP;
            }
            slide += TOWER_SEAT_STEP;
        }
        None
    }

    /// Посадка глав: центр пучка и разлёт малых глав, ноль — одна глава. То же
    /// правило и та же причина, что у [`Plan::west_tower`]: у крестового плана
    /// восточная доля `min_area_rect` приходится на апсиду и на воздух за ней,
    /// и барабаны малых глав вырастали из стен (Тула, way 234273451 — четыре
    /// главы из пяти висели над землёй за апсидой).
    ///
    /// Пучок в полный разлёт отодвигается от алтаря на запад шагами
    /// `TOWER_SEAT_STEP`, но не дальше `room` — там стоит колокольня, и венцом
    /// он был бы ей, а не храму; не встал — разлёт сжимается и поиск идёт
    /// заново, а в последнюю очередь остаётся одна глава. Порядок перебора и
    /// здесь правило раскладки: пятиглавие важнее места, место важнее
    /// восточного конца. `None` — не встала и одна глава; что с этим делать,
    /// решает вызывающий.
    ///
    /// Глава стоит не на контуре, а на **площадке кровли**, и проверяется
    /// поэтому на `radius + landmark_inset` от края: площадка вальмы вдвинута
    /// от карниза на вылет ската, и барабан у самого края стоял бы на скате и
    /// свешивался бы с кровли — две восточные главы у way 234273451 так и
    /// наезжали на скат.
    fn dome_seat(
        &self,
        building: &PolyArea,
        core: Vec2,
        radius: f32,
        spread: f32,
        room: f32,
    ) -> Option<(Vec2, f32)> {
        let inset = landmark_inset(building);
        let stands = |at: Vec2, radius: f32| {
            disc(at, radius + inset, DOME_SEAT_PROBES)
                .into_iter()
                .all(|point| point_in_area(point, building))
        };
        let mut offset = spread;
        loop {
            let mut slide = 0.0;
            while slide <= room {
                let at = core - self.axis * slide;
                let all = stands(at, radius)
                    && (offset == 0.0
                        || minor_domes(self, at, offset)
                            .into_iter()
                            .all(|minor| stands(minor, radius * MINOR_DOME_SHARE)));
                if all {
                    return Some((at, offset));
                }
                slide += TOWER_SEAT_STEP;
            }
            if offset == 0.0 {
                return None;
            }
            offset = match offset - TOWER_SEAT_STEP >= radius * DOME_SPREAD_MIN {
                true => offset - TOWER_SEAT_STEP,
                false => 0.0,
            };
        }
    }
}

/// Сдвиги поперёк оси, от нуля наружу и не дальше `room`: середина торца
/// пробуется первой, а дальше — всё более смещённые посадки.
fn nudges(room: f32) -> impl Iterator<Item = f32> {
    let steps = (room / TOWER_SEAT_STEP) as u32;
    std::iter::once(0.0).chain(
        (1..=steps)
            .map(|step| step as f32 * TOWER_SEAT_STEP)
            .flat_map(|shift| [shift, -shift]),
    )
}

/// Радиус малой главы пятиглавия в радиусах большой.
const MINOR_DOME_SHARE: f32 = 0.5;

/// Четыре малые главы вокруг большой — по углам квадрата со стороной `2 · offset`.
fn minor_domes(plan: &Plan, core: Vec2, offset: f32) -> [Vec2; 4] {
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(u, v)| core + plan.axis * (u * offset) + plan.perp * (v * offset))
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
    /// Храмы, у которых размечена **своя колокольня** ([`is_standalone_tower`]
    /// того же посева): корабельную башню такому храму от себя ставить
    /// незачем — у кремлёвского собора Тулы она вставала в десяти метрах от
    /// настоящей, и над собором торчали два шатра.
    towered: HashSet<u32>,
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
        let mut towered = HashSet::new();
        for (index, building) in buildings.iter().enumerate() {
            let BuildingUse::Church(sacred) = building.building_use else {
                continue;
            };
            if is_standalone_tower(building) && sacred.complex != 0 {
                towered.insert(sacred.complex);
            }
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
        Self {
            raised,
            domed,
            towered,
        }
    }

    /// С какой высоты начинается приподнятая часть; `None` — дом стоит на земле
    /// и рисуется коробкой.
    pub(super) fn raised(&self, index: usize) -> Option<f32> {
        self.raised.get(&index).copied()
    }

    /// Дом, у которого коробки нет — весь он венец: приподнятая часть
    /// (барабан с главой) и отдельно стоящая колокольня
    /// ([`is_standalone_tower`]). Слои по этому не кладут ни стен, ни кровли,
    /// ни тени коробки — тень даёт венец ([`Sanctuary::shadow_casters`]) — и не
    /// кладут на такой дом тени соседей.
    pub(super) fn boxless(&self, index: usize, building: &PolyArea) -> bool {
        self.raised(index).is_some() || is_standalone_tower(building)
    }

    /// Венец дома с его посадкой: `lift` — подъём карниза этого дома в текущем
    /// режиме. У дома без коробки ([`Sanctuary::boxless`]) посадка нулевая, а
    /// высота начала уже в самом элементе, — она меряется от земли, а не от
    /// своего карниза.
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
        let own = match building.building_use {
            BuildingUse::Church(sacred) => Own {
                domes: !self.domed.contains(&sacred.complex),
                tower: !self.towered.contains(&sacred.complex),
            },
            _ => Own::default(),
        };
        let eave = match is_standalone_tower(building) {
            true => Vec2::ZERO,
            false => lift,
        };
        crowns_with(building, wall, roof, own)
            .into_iter()
            .map(|crown| (crown, eave))
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
        let eave = match self.boxless(index, building) {
            true => 0.0,
            false => height_or_default(building),
        };
        self.crowns(index, building, Srgba::WHITE, Srgba::WHITE, Vec2::ZERO)
            .iter()
            .map(|(crown, _)| {
                let outline = match *crown {
                    Crown::Dome { at, radius, .. } => disc(at, radius, DOME_SIDES),
                    Crown::Tower { at, axis, size, .. } => rect(at, axis, size).to_vec(),
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

/// Цвет глав — из разметки ([`tagged_dome`]), а без неё по посеву **храма**
/// ([`look_seed`]): у частей одного собора главы одного цвета.
fn dome_color(building: &PolyArea, faith: Faith) -> Srgba {
    if let Some(tagged) = tagged_dome(building) {
        return tagged;
    }
    let domes = dome_palette(faith);
    domes[(look_seed(building) >> 18) as usize % domes.len()].to_srgba()
}

/// Что храм ставит **от себя**, а что у него уже размечено частями
/// ([`Sanctuary`]): главы — барабанами, колокольня — своим контуром. По
/// умолчанию — всё своё: храм, стоящий один.
#[derive(Clone, Copy, Debug)]
pub(super) struct Own {
    pub(super) domes: bool,
    pub(super) tower: bool,
}

impl Default for Own {
    fn default() -> Self {
        Self {
            domes: true,
            tower: true,
        }
    }
}

/// Венец храма: `wall` и `roof` — цвета, которые дому уже выбрали стена и
/// кровля (барабан красится стеной, шатёр колокольни — кровлей), `own` — что
/// храм ставит от себя ([`Sanctuary`] знает, что у него размечено частями).
/// Не храм и пристройка — пусто.
pub(super) fn crowns_with(building: &PolyArea, wall: Srgba, roof: Srgba, own: Own) -> Vec<Crown> {
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
        (Faith::Orthodox | Faith::Western | Faith::Unknown, SacredForm::Tower) => {
            out.push(standalone_tower(
                building, &plan, sacred, seed, wall, roof, dome_color,
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
            &mut out, building, &plan, seed, own, rise, wall, roof, dome_color,
        ),
        (Faith::Western | Faith::Unknown, _) => {
            let wanted = (plan.width * 0.5).clamp(4.0, 10.0);
            if own.tower
                && plan.length >= WESTERN_TOWER_LENGTH_MIN
                && let Some((at, axis, size)) = plan.tower_seat(building, wanted)
            {
                let height = (plan.width * 0.8 + 6.0).clamp(10.0, 30.0);
                out.push(Crown::Tower {
                    at,
                    axis,
                    size,
                    base: 0.0,
                    height,
                    spire: (size.min_element() * 2.4).clamp(8.0, 32.0),
                    tiers: tier_count(height, size),
                    top: TowerTop::Spire,
                    wall,
                    roof,
                    cap: None,
                });
            }
        }
        (Faith::Muslim, _) => {
            let radius = (plan.width.min(plan.length) * 0.3).clamp(2.0, 14.0);
            if own.domes {
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
            if own.domes && plan.area >= SYNAGOGUE_DOME_AREA {
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

/// Православный храм: главы и колокольня. Без `own.domes` — одна колокольня:
/// главы у храма размечены частями; без `own.tower` — одни главы: колокольня
/// стоит рядом своим контуром.
#[allow(clippy::too_many_arguments)]
fn orthodox_nave(
    out: &mut Vec<Crown>,
    building: &PolyArea,
    plan: &Plan,
    seed: u32,
    own: Own,
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
    let seat = (ship && own.tower)
        .then(|| plan.tower_seat(building, (plan.width * 0.6).clamp(4.0, 9.0)))
        .flatten();
    // восточный край столпа по оси плана: у него своя ось, так что мерить
    // половиной стороны нельзя — только по углам
    let tower_east = seat.map(|(at, axis, size)| {
        rect(at, axis, size)
            .iter()
            .map(|corner| (*corner - plan.center).dot(plan.axis))
            .fold(f32::MIN, f32::max)
    });
    let (core, core_length) = if let Some((at, axis, size)) = seat {
        let height = (plan.width * 1.2).clamp(8.0, 28.0);
        out.push(Crown::Tower {
            at,
            axis,
            size,
            base: 0.0,
            height,
            spire: size.min_element() * 1.5,
            tiers: tier_count(height, size),
            top: tower_top(Faith::Orthodox, seed),
            wall,
            roof,
            cap: Some(dome_color),
        });
        // ядро — восточная часть за колокольней и трапезной; колокольня могла
        // вдвинуться от торца, и тогда она занимает больше своей стороны
        let taken = tower_east.unwrap_or_default() + plan.length / 2.0;
        let core_length = (plan.length - taken) * 0.6;
        (
            plan.center + plan.axis * (plan.length / 2.0 - core_length / 2.0),
            core_length,
        )
    } else {
        (plan.center, plan.length)
    };
    // главы, размеченные частями, заменяют все свои: малые главы вокруг них
    // встали бы вперемешку с настоящими и слиплись бы с ними парами
    if !own.domes {
        return;
    }
    let side = core_length.min(plan.width);
    let radius = (side * 0.2).clamp(1.6, 6.0);
    let five = side >= FIVE_DOMES_MIN_SIDE && (seed >> 11) % 10 < FIVE_DOMES_SHARE;
    let spread = match five {
        true => side * 0.28,
        false => 0.0,
    };
    // на запад пучку — до колокольни, и ни шагом дальше
    let room = match tower_east {
        Some(east) => (core - plan.center).dot(plan.axis) - east,
        None => plan.length,
    };
    // не нашлось места и одной главе — пусть стоит где стояла: храм без главы
    // читается хуже, чем глава над краем кровли
    let (core, offset) = plan
        .dome_seat(building, core, radius, spread, room)
        .unwrap_or((core, 0.0));
    if offset > 0.0 {
        for at in minor_domes(plan, core, offset) {
            out.push(dome(at, radius * MINOR_DOME_SHARE, radius * 0.9));
        }
    }
    out.push(dome(core, radius, radius * 1.1));
}

/// Отдельно стоящая колокольня — `building=bell_tower`, `tower:type=bell_tower`
/// — православная или западная: коробки у неё нет, весь дом — венец
/// ([`standalone_tower`]). До этого она рисовалась коробкой на всю высоту с
/// храмовыми окнами по всем восьмидесяти метрам и шатром сверху — Всехсвятская
/// колокольня Тулы (82 м) выходила розовой девятиэтажкой. Минарет остаётся
/// коробкой с [`Crown::Minaret`] сверху, восточная и синагогальная башни —
/// коробкой под шатром.
pub(super) fn is_standalone_tower(building: &PolyArea) -> bool {
    matches!(
        building.building_use,
        BuildingUse::Church(Sacred {
            form: SacredForm::Tower,
            faith: Faith::Orthodox | Faith::Western | Faith::Unknown,
            ..
        })
    )
}

/// Колокольня, стоящая отдельно, целиком: столп по прямоугольнику её контура
/// от земли и шатёр или шпиль над ним. Высота из OSM — **с шпилем** (у
/// кремлёвской колокольни Тулы 70 м, у Всехсвятской 82), поэтому столпу
/// достаётся `TOWER_PILLAR_SHARE` её; додуманная высота (`heights.rs`) — до
/// карниза яруса звона, и шпиль идёт сверх неё.
fn standalone_tower(
    building: &PolyArea,
    plan: &Plan,
    sacred: Sacred,
    seed: u32,
    wall: Srgba,
    roof: Srgba,
    dome_color: Srgba,
) -> Crown {
    let size = Vec2::new(plan.length, plan.width);
    let total = height_or_default(building);
    let (height, spire) = match building.height {
        Some(_) => (
            total * TOWER_PILLAR_SHARE,
            total * (1.0 - TOWER_PILLAR_SHARE),
        ),
        None => (total, size.min_element() * 1.5),
    };
    let cap =
        (sacred.faith == Faith::Orthodox).then(|| tagged_roof(building).unwrap_or(dome_color));
    Crown::Tower {
        at: plan.center,
        axis: plan.axis,
        size,
        base: 0.0,
        height,
        spire,
        tiers: tier_count(height, size),
        top: tower_top(sacred.faith, seed),
        wall,
        roof,
        cap,
    }
}

/// Ярусов у столпа высотой `height` над нижним ярусом `size`: на каждые
/// `TIER_ASPECT` узких сторон **столпа** (не шире `SHAFT_SIDE_MAX`) по ярусу,
/// от одного до `TIERS_MAX`.
fn tier_count(height: f32, size: Vec2) -> u8 {
    let shaft = size.min_element().clamp(1.0, SHAFT_SIDE_MAX);
    let tiers = (height / (shaft * TIER_ASPECT)).round();
    (tiers as u8).clamp(1, TIERS_MAX)
}

/// Ярус над ярусом `size`: у́же в `TIER_SHRINK` и не шире столпа.
fn next_tier(size: Vec2) -> Vec2 {
    (size * TIER_SHRINK).min(Vec2::splat(SHAFT_SIDE_MAX))
}

/// Чем кончать столп: у православной колокольни шатёр чаще шпиля, по посеву
/// храма; западная башня — всегда шпиль.
fn tower_top(faith: Faith, seed: u32) -> TowerTop {
    match faith {
        Faith::Orthodox if (seed >> 14) % 10 >= SPIRE_SHARE_OF_10 => TowerTop::Tent,
        _ => TowerTop::Spire,
    }
}

/// Размер верхнего яруса: нижний, сжатый [`next_tier`] на каждый ярус выше.
fn top_tier(size: Vec2, tiers: u8) -> Vec2 {
    (1..tiers.clamp(1, TIERS_MAX)).fold(size, |size, _| next_tier(size))
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
            size,
            tiers,
            top,
            ..
        } => {
            let cap = cap.map_or(0.0, |_| {
                cap_radius(top_tier(size, tiers), top) * (CAP_DRUM + Profile::Onion.height())
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
/// Ломтиков у главы. Шестнадцати не хватало: каждый ломтик — плоский диск
/// своего тона, и на большой главе (радиус 6 м, вытяжка 2.6) ступени между
/// ними читались полосами поперёк луковицы.
const DOME_SLICES: usize = 32;
/// Карниз барабана под главой: насколько шире барабана и какой высоты, м.
const DRUM_CORNICE_REACH: f32 = 1.12;
const DRUM_CORNICE_HEIGHT: f32 = 0.35;
/// Граней у ствола минарета.
const SHAFT_SIDES: usize = 12;
/// Подъём конуса минарета в радиусах ствола.
const CONE_RISE: f32 = 3.0;
/// Сколько от главы остаётся в тени и сколько добавляет свет: тон ломтика —
/// `AMBIENT + DIFFUSE × ламберт`.
const AMBIENT: f32 = 0.52;
const DIFFUSE: f32 = 0.62;
/// Блик металла главы: доля смеси к белому на ламберте в единицу. Золото
/// узнают по блику, и при 0.35 глава читалась крашеной, а не позолоченной.
const SPECULAR: f32 = 0.55;
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
                    // карниз барабана — белёный поясок, на котором глава сидит:
                    // без него луковица вырастает из трубы
                    let cornice = drum.min(DRUM_CORNICE_HEIGHT);
                    push_shaft(
                        builder,
                        seat + up(lean, drum - cornice),
                        drum_radius * DRUM_CORNICE_REACH,
                        cornice,
                        drum_color.mix(&Srgba::WHITE, CORNICE_LIGHTEN),
                        lean,
                        false,
                    );
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
                size,
                base,
                height,
                spire,
                tiers,
                top,
                wall,
                roof,
                cap,
            } => {
                let seat = at + eave + up(lean, base);
                let pillar = Pillar {
                    axis,
                    size,
                    height,
                    spire,
                    tiers,
                    top,
                    wall,
                    roof,
                };
                push_tower(builder, seat, &pillar, lean);
                if let Some(color) = cap {
                    let radius = cap_radius(top_tier(size, tiers), top);
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

/// Радиус маковки на вершине колокольни — по узкой стороне **верхнего** яруса:
/// шатёр сходится в точку над ней, и маковка по широкой свесилась бы с его
/// граней. На шпиле — шар в `SPIRE_BALL` от неё.
fn cap_radius(size: Vec2, top: TowerTop) -> f32 {
    let radius = (size.min_element() * 0.14).clamp(0.6, 2.0);
    match top {
        TowerTop::Tent => radius,
        TowerTop::Spire => radius * SPIRE_BALL,
    }
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

/// Столп колокольни: то из [`Crown::Tower`], что нужно, чтобы его нарисовать.
struct Pillar {
    axis: Vec2,
    size: Vec2,
    height: f32,
    spire: f32,
    tiers: u8,
    top: TowerTop,
    wall: Srgba,
    roof: Srgba,
}

/// Столп колокольни ярусами и его завершение. Ярусы кладутся снизу вверх:
/// стены яруса, на них карниз (он шире яруса — вылет, который и читается
/// ярусом), на карнизе следующий ярус у́же. Верхний ярус — звон: две высокие
/// арки на каждой видимой стене; нижние ярусы — по окну. Дальше по
/// [`TowerTop`]: шатёр гранями от верхнего яруса к вершине, или карниз, фонарь
/// и тонкий шпиль от его крыши.
fn push_tower(builder: &mut MeshBuilder, seat: Vec2, pillar: &Pillar, lean: Option<Lean>) {
    let tiers = usize::from(pillar.tiers.clamp(1, TIERS_MAX));
    let shares: f32 = TIER_SHARES[..tiers].iter().sum();
    let cornices = (tiers - 1) as f32 * CORNICE_HEIGHT;
    let walls = (pillar.height - cornices).max(1.0);
    let cornice_color = pillar.wall.mix(&Srgba::WHITE, CORNICE_LIGHTEN);
    let mut floor = seat;
    let mut size = pillar.size;
    let mut heights: Vec<f32> = TIER_SHARES[..tiers]
        .iter()
        .map(|share| walls * share / shares)
        .collect();
    // широкое основание — паперти и палаты нижнего яруса — не выше
    // `BASE_TIER_MAX`: лишнее уходит в столп над ним
    if tiers > 1 && pillar.size.min_element() > SHAFT_SIDE_MAX && heights[0] > BASE_TIER_MAX {
        let excess = heights[0] - BASE_TIER_MAX;
        heights[0] = BASE_TIER_MAX;
        let upper: f32 = heights[1..].iter().sum();
        for height in &mut heights[1..] {
            *height += excess * *height / upper;
        }
    }
    for (tier, height) in heights.into_iter().enumerate() {
        let ring = rect(floor, pillar.axis, size);
        let rise = up(lean, height);
        push_tier_walls(
            builder,
            &ring,
            rise,
            height,
            pillar.wall,
            lean,
            tier + 1 == tiers,
        );
        floor += rise;
        if tier + 1 < tiers {
            floor = push_cornice(builder, floor, pillar.axis, size, cornice_color, lean);
            size = next_tier(size);
        }
    }
    let ring = rect(floor, pillar.axis, size);
    match pillar.top {
        TowerTop::Tent => {
            push_cone(
                builder,
                &ring,
                floor + up(lean, pillar.spire),
                pillar.roof,
                lean,
            );
        }
        TowerTop::Spire => {
            let roof = push_cornice(builder, floor, pillar.axis, size, cornice_color, lean);
            let radius = size.min_element() * LANTERN_RADIUS;
            let lantern = (size.min_element() * LANTERN_HEIGHT)
                .min((pillar.spire - CORNICE_HEIGHT) * 0.5)
                .max(0.0);
            push_shaft(
                builder,
                roof,
                radius,
                lantern,
                pillar.wall,
                lean,
                lantern >= 1.5,
            );
            let foot = disc(roof + up(lean, lantern), radius * SPIRE_FOOT, SHAFT_SIDES);
            push_cone(
                builder,
                &foot,
                floor + up(lean, pillar.spire),
                pillar.roof,
                lean,
            );
        }
    }
}

/// Стены одного яруса высотой `height` м: видимые грани с тоном по свету и
/// проёмы — арки звона на верхнем ярусе, окно (два на длинной стене) на
/// остальных, размером в метрах ([`BELFRY_ARCH`], [`TIER_WINDOW`]).
#[allow(clippy::too_many_arguments)]
fn push_tier_walls(
    builder: &mut MeshBuilder,
    ring: &[Vec2; 4],
    rise: Vec2,
    height: f32,
    wall: Srgba,
    lean: Option<Lean>,
    belfry: bool,
) {
    let Some(lean) = lean else {
        return;
    };
    let opening: LinearRgba = OPENING_COLOR.into();
    for index in 0..4 {
        let (a, b) = (ring[index], ring[(index + 1) % 4]);
        let edge = b - a;
        let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero();
        if outward.dot(lean.dir()) > 0.0 {
            continue;
        }
        let (bottom, top) = wall_colors(wall, a, b, lean.dir());
        builder.push_quad_gradient([a, b, b + rise, a + rise], [bottom, bottom, top, top]);
        let length = edge.length();
        if length <= 0.0 || height <= 0.0 {
            continue;
        }
        let (size, count, low): (Vec2, usize, f32) = match belfry {
            true => (
                BELFRY_ARCH,
                ((length / BELFRY_PITCH) as usize).clamp(1, BELFRY_ARCHES_MAX),
                0.2,
            ),
            false => (
                TIER_WINDOW,
                1 + usize::from(length >= TWO_WINDOWS_FROM),
                0.4,
            ),
        };
        // ширина — в долях стены, высота — в долях яруса, обе не больше того,
        // что стена и ярус вмещают
        let wide = (size.x / length).min(OPENING_SHARE_MAX);
        let tall = (size.y / height).min(0.6);
        let low = low.min(1.0 - tall - 0.08);
        for slot in 0..count {
            let centre = (slot as f32 + 0.5) / count as f32;
            let (left, right) = (
                a.lerp(b, centre - wide / 2.0),
                a.lerp(b, centre + wide / 2.0),
            );
            let (bottom, top) = (rise * low, rise * (low + tall));
            builder.push_quad(
                [left + bottom, right + bottom, right + top, left + top],
                opening,
            );
        }
    }
}

/// Карниз над ярусом `size` с центром `floor`: плита шире яруса на
/// `CORNICE_REACH`, её видимые бока и верх. Возвращает центр её верха — на нём
/// стоит следующий ярус или фонарь.
fn push_cornice(
    builder: &mut MeshBuilder,
    floor: Vec2,
    axis: Vec2,
    size: Vec2,
    color: Srgba,
    lean: Option<Lean>,
) -> Vec2 {
    let ring = rect(floor, axis, size + Vec2::splat(2.0 * CORNICE_REACH));
    let rise = up(lean, CORNICE_HEIGHT);
    if let Some(lean) = lean {
        for index in 0..4 {
            let (a, b) = (ring[index], ring[(index + 1) % 4]);
            let edge = b - a;
            let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero();
            if outward.dot(lean.dir()) > 0.0 {
                continue;
            }
            builder.push_quad(
                [a, b, b + rise, a + rise],
                shade_by_light(color, outward, 0.2, 0.25).into(),
            );
        }
    }
    let top = ring.map(|corner| corner + rise);
    builder.push_convex(&top, color.mix(&Srgba::WHITE, 0.06).into());
    floor + rise
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

/// Прямоугольник `size` (вдоль `axis` × поперёк) вокруг `at`, против часовой.
fn rect(at: Vec2, axis: Vec2, size: Vec2) -> [Vec2; 4] {
    let perp = Vec2::new(-axis.y, axis.x);
    let (u, v) = (axis * (size.x / 2.0), perp * (size.y / 2.0));
    [at - u - v, at + u - v, at + u + v, at - u + v]
}
