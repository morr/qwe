//! Столпы и шпили: отдельно стоящая колокольня ([`standalone_tower`]), ярусы
//! столпа ([`tier_count`], [`next_tier`]) и его отрисовка — стены ярусов с
//! арками звона ([`push_tier_walls`]), карнизы ([`push_cornice`]), шатёр или
//! фонарь со шпилем ([`push_tower`]), минарет ([`push_minaret`]). Посадка
//! столпа на храме — в родителе ([`super::Plan::tower_seat`]).

use bevy::color::Mix;
use bevy::prelude::*;

use super::{
    CONE_RISE, CORNICE_LIGHTEN, Crown, HouseFootprint, Lean, OPENING_COLOR, Plan, SHAFT_SIDES,
    TIERS_MAX, TowerTop, disc, push_shaft, rect, shade_by_light, tagged_roof, up,
};
use crate::map::buildings::heights::height_of;
use crate::map::buildings::layers::wall_colors;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{Faith, PolyArea, Sacred};

/// Сколько православных колоколен из десяти — со шпилем, а не с шатром.
const SPIRE_SHARE_OF_10: u32 = 4;
/// Ярусы столпа: ярус на каждые столько узких сторон высоты, не больше трёх.
/// Кремлёвская колокольня Тулы (12 × 12 м, столп ~50 м) — три яруса, столп
/// корабля 6 × 22 м — два, часовенная башенка — один.
const TIER_ASPECT: f32 = 1.6;
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
/// Доля высоты колокольни из OSM, которая столп; остальное — шатёр или шпиль.
/// У кремлёвской колокольни Тулы 70 м по тегу — это с шпилем.
const TOWER_PILLAR_SHARE: f32 = 0.72;
/// Фонарь под шпилем: радиус и высота в долях узкой стороны верхнего яруса.
const LANTERN_RADIUS: f32 = 0.28;
const LANTERN_HEIGHT: f32 = 0.45;
/// Основание шпиля в долях радиуса фонаря — он у́же фонаря, на его крыше.
const SPIRE_FOOT: f32 = 0.8;

/// Колокольня, стоящая отдельно, целиком: столп по прямоугольнику её контура
/// от земли и шатёр или шпиль над ним. Высота из OSM — **с шпилем** (у
/// кремлёвской колокольни Тулы 70 м, у Всехсвятской 82), поэтому столпу
/// достаётся `TOWER_PILLAR_SHARE` её; додуманная высота (`heights.rs`) — до
/// карниза яруса звона, и шпиль идёт сверх неё.
#[allow(clippy::too_many_arguments)]
pub(super) fn standalone_tower(
    building: &PolyArea,
    plan: &Plan,
    sacred: Sacred,
    seed: u32,
    wall: Srgba,
    roof: Srgba,
    dome_color: Srgba,
    fp: Option<HouseFootprint>,
) -> Crown {
    let size = Vec2::new(plan.length, plan.width);
    let total = height_of(building, fp);
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
pub(super) fn tier_count(height: f32, size: Vec2) -> u8 {
    let shaft = size.min_element().clamp(1.0, SHAFT_SIDE_MAX);
    let tiers = (height / (shaft * TIER_ASPECT)).round();
    (tiers as u8).clamp(1, TIERS_MAX)
}

/// Ярус над ярусом `size`: у́же в `TIER_SHRINK` и не шире столпа.
pub(super) fn next_tier(size: Vec2) -> Vec2 {
    (size * TIER_SHRINK).min(Vec2::splat(SHAFT_SIDE_MAX))
}

/// Чем кончать столп: у православной колокольни шатёр чаще шпиля, по посеву
/// храма; западная башня — всегда шпиль.
pub(super) fn tower_top(faith: Faith, seed: u32) -> TowerTop {
    match faith {
        Faith::Orthodox if (seed >> 14) % 10 >= SPIRE_SHARE_OF_10 => TowerTop::Tent,
        _ => TowerTop::Spire,
    }
}

/// Столп колокольни: то из [`Crown::Tower`], что нужно, чтобы его нарисовать.
pub(super) struct Pillar {
    pub(super) axis: Vec2,
    pub(super) size: Vec2,
    pub(super) height: f32,
    pub(super) spire: f32,
    pub(super) tiers: u8,
    pub(super) top: TowerTop,
    pub(super) wall: Srgba,
    pub(super) roof: Srgba,
}

/// Столп колокольни ярусами и его завершение. Ярусы кладутся снизу вверх:
/// стены яруса, на них карниз (он шире яруса — вылет, который и читается
/// ярусом), на карнизе следующий ярус у́же. Верхний ярус — звон: две высокие
/// арки на каждой видимой стене; нижние ярусы — по окну. Дальше по
/// [`TowerTop`]: шатёр гранями от верхнего яруса к вершине, или карниз, фонарь
/// и тонкий шпиль от его крыши.
pub(super) fn push_tower(
    builder: &mut MeshBuilder,
    seat: Vec2,
    pillar: &Pillar,
    lean: Option<Lean>,
) {
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
pub(super) fn push_minaret(
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
