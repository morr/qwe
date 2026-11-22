//! Главы храмов: профиль ([`Profile`]) и его образующая ([`onion_radius`]),
//! цвет из разметки или палитры ([`dome_color`]), барабаны ([`raised_drum`],
//! [`drum_cupola`], [`is_drum`]) и отрисовка главы стопкой ломтиков
//! ([`push_dome`]). План храма и венец в сборе — в родителе.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::color::Mix;
use bevy::prelude::*;

use super::{Crown, DOME_SIDES, HouseFootprint, Lean, Light, Plan, TowerTop, up};
use crate::map::buildings::heights::height_of;
use crate::map::buildings::material::look_seed;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{BuildingUse, Faith, PolyArea, Sacred, SacredForm, srgba_of};

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
const WESTERN_DOMES: [Color; 2] = [Color::srgb(0.40, 0.57, 0.51), Color::srgb(0.58, 0.60, 0.62)];
const MUSLIM_DOMES: [Color; 4] = [
    Color::srgb(0.24, 0.56, 0.56),
    Color::srgb(0.22, 0.50, 0.36),
    Color::srgb(0.60, 0.62, 0.64),
    Color::srgb(0.84, 0.68, 0.30),
];
const JEWISH_DOMES: [Color; 2] = [Color::srgb(0.40, 0.56, 0.50), Color::srgb(0.62, 0.64, 0.66)];
const EASTERN_DOMES: [Color; 2] = [Color::srgb(0.86, 0.68, 0.28), Color::srgb(0.90, 0.89, 0.86)];

fn dome_palette(faith: Faith) -> &'static [Color] {
    match faith {
        Faith::Orthodox => &ORTHODOX_DOMES,
        Faith::Western | Faith::Unknown => &WESTERN_DOMES,
        Faith::Muslim => &MUSLIM_DOMES,
        Faith::Jewish => &JEWISH_DOMES,
        Faith::Eastern => &EASTERN_DOMES,
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

/// Часть храма не шире этого — барабан под главой, и кровли у неё не видно:
/// её закрывает сама глава, м. Шире — мапер пометил главой весь четверик, и
/// рисуется он храмом с главами.
const DOME_PART_MAX_SIDE: f32 = 14.0;

/// Барабан под главой: часть с `roof:shape=onion|dome`, узкая настолько, что
/// сама она и есть барабан.
pub(super) fn is_drum(sacred: Sacred, building: &PolyArea, fp: Option<HouseFootprint>) -> bool {
    sacred.form == SacredForm::Dome
        && Plan::of(building, fp).is_some_and(|plan| plan.width <= DOME_PART_MAX_SIDE)
}

/// Профиль главы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::map::buildings) enum Profile {
    /// Луковица: пузо шире барабана и острая маковка.
    Onion,
    /// Полусфера мечети, синагоги, барочного собора.
    Hemisphere,
}

impl Profile {
    /// Высота главы в её радиусах.
    pub(super) fn height(self) -> f32 {
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
pub(super) const ONION_NECK: f32 = 0.72;

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

/// Приподнятая часть: барабан от высоты начала до верха части и глава на нём.
/// Верх части в OSM — это маковка (`height` вместе с `roof:height`), поэтому
/// барабан кончается там, где начинается глава.
pub(super) fn raised_drum(
    building: &PolyArea,
    floor: f32,
    wall: Srgba,
    fp: Option<HouseFootprint>,
) -> Option<Crown> {
    let BuildingUse::Church(sacred) = building.building_use else {
        return None;
    };
    let plan = Plan::of(building, fp)?;
    let (profile, radius) = drum_cupola(sacred.faith, &plan);
    let drum = (height_of(building, fp) - floor - radius * profile.height()).max(radius * 0.6);
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
pub(super) fn drum_cupola(faith: Faith, plan: &Plan) -> (Profile, f32) {
    let profile = match faith {
        Faith::Orthodox => Profile::Onion,
        _ => Profile::Hemisphere,
    };
    (profile, (plan.width * 0.45).clamp(1.2, 6.0))
}

/// Цвет глав — из разметки ([`tagged_dome`]), а без неё по посеву **храма**
/// ([`look_seed`]): у частей одного собора главы одного цвета.
pub(super) fn dome_color(building: &PolyArea, faith: Faith) -> Srgba {
    if let Some(tagged) = tagged_dome(building) {
        return tagged;
    }
    let domes = dome_palette(faith);
    domes[(look_seed(building) >> 18) as usize % domes.len()].to_srgba()
}

/// Во сколько раз луковица и её барабан вытянуты вверх против остального
/// дома. Подъём 2.5D сжимает высоту втрое (`EXTRUDE_SCALE`), и честная
/// луковица на экране выходила шаром: силуэт главы держится ровно на её
/// вытянутости. Полусфера вытянута слабее — на полной вытяжке купол мечети
/// становился яйцом. Тень главы меряется настоящими метрами ([`super::top`]) —
/// вытягивается рисунок, а не храм.
const ONION_STRETCH: f32 = 2.6;
const HEMISPHERE_STRETCH: f32 = 1.4;

impl Profile {
    pub(super) fn stretch(self) -> f32 {
        match self {
            Self::Onion => ONION_STRETCH,
            Self::Hemisphere => HEMISPHERE_STRETCH,
        }
    }
}
/// Ломтиков у главы. Шестнадцати не хватало: каждый ломтик — плоский диск
/// своего тона, и на большой главе (радиус 6 м, вытяжка 2.6) ступени между
/// ними читались полосами поперёк луковицы.
const DOME_SLICES: usize = 32;

/// Маковка на шпиле — шар, а не глава: доля от маковки шатра.
const SPIRE_BALL: f32 = 0.5;

/// Радиус маковки на вершине колокольни — по узкой стороне **верхнего** яруса:
/// шатёр сходится в точку над ней, и маковка по широкой свесилась бы с его
/// граней. На шпиле — шар в `SPIRE_BALL` от неё.
pub(super) fn cap_radius(size: Vec2, top: TowerTop) -> f32 {
    let radius = (size.min_element() * 0.14).clamp(0.6, 2.0);
    match top {
        TowerTop::Tent => radius,
        TowerTop::Spire => radius * SPIRE_BALL,
    }
}

/// Глава стопкой ломтиков от `seat` вверх.
pub(super) fn push_dome(
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
