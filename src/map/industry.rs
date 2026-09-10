//! Промзона: цилиндры (резервуары, силосы, заводские трубы, водонапорные
//! башни, газгольдеры) и надземные трубопроводы.
//!
//! На снимке промзону выдают не корпуса — корпуса это те же коробки, что и
//! везде, — а **круглые пятна и их тени**. Резервуар нефтебазы читается как
//! круг светлого металла, а заводская труба — вообще только тенью: сам кружок
//! в три метра теряется, а тень от него уходит на полста метров через всю
//! площадку, и именно по ней глаз опознаёт трубу.
//!
//! Поэтому цилиндр рисуется тремя слоями, как дом: тень, видимая стена, верх, —
//! и «как дом» здесь буквально: тень его есть ровно в тех режимах
//! `BuildingHeightMode`, где рисуется домовая (`casts_shadows`), а стена — там,
//! где дом накренён.
//! Стена берётся тем же отклонением верха ([`drawn_lift`]), что и у домов
//! (включая обрезку высоты: без неё труба ложится на карту трубой), — иначе
//! цилиндр стоял бы плоским кругом среди накренённых коробок, — и
//! красится посторонне ([`shade_by_light`]): каждая грань многоугольника со
//! своим тоном, отчего цилиндр читается круглым, а не гранёным.
//!
//! Второе, что выдаёт русскую промзону с воздуха, — **надземная теплотрасса**:
//! светлая нитка на опорах, которая идёт через дворы напрямик, ныряет над
//! проездом и не считается ни с домами, ни с оградами. Рисуется она как
//! ограда — линия и её тень, — только тень длиннее (опоры в три метра против
//! двухметрового забора) и слой выше: труба перешагивает забор, а не упирается
//! в него.
//!
//! **Навмеша не касается**, как ограды и деревья: десяток цилиндров на город
//! стоят внутри промзон, куда толпа не ходит, а перегородить ими проход было
//! бы дороже, чем нарисовать.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;

use crate::map::buildings::{SHADOW_LENGTH_RANGE, drawn_lift, shade_by_light};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::{MapData, Structure, StructureKind};
use crate::map::surface::{self, LayerMaterial};
use crate::map::{BuildingHeightMode, SHADOW_COLOR, shadow_dir, shadow_length_scale, sun_stretch};
use crate::settings::{Z_INDUSTRY, Z_INDUSTRY_SHADOW, Z_INDUSTRY_WALL, Z_PIPE, Z_PIPE_SHADOW};

/// Сторон в круге. Двадцать четыре: у резервуара в двадцать метров это грань
/// в пять метров, и на любом зуме, где резервуар вообще виден, круг остаётся
/// кругом.
const CIRCLE_SIDES: usize = 24;

/// Насколько грань, повёрнутая к свету, светлее базового тона стены, и
/// насколько отвёрнутая — темнее. Сильнее, чем у плоской стены дома
/// (`WALL_LIT_MIX` 0.18/0.22): у цилиндра градиент идёт по всей видимой
/// половине, и без запаса он вырождается в ровную заливку.
const WALL_LIT_MIX: f32 = 0.26;
const WALL_SHADED_MIX: f32 = 0.26;

/// Кромка: тёмное кольцо по краю верха. У резервуара это борт, у трубы —
/// толщина её стенки, и без него круг выглядит наклейкой.
const RIM_MIX: f32 = 0.28;
const RIM_SHARE: f32 = 0.1;
const RIM_RANGE: std::ops::RangeInclusive<f32> = 0.25..=1.0;

/// Высота надземного трубопровода над землёй, м: теплотрассу кладут на опоры
/// в рост человека, а через проезд поднимают вдвое выше. Тень по ней считает
/// тот же котангенс, что у оград и домов.
const PIPE_HEIGHT: f32 = 3.0;

/// Оцинкованный кожух изоляции — светлее всего, что под ним: на снимке
/// теплотрасса тянется через двор светлой ниткой.
const PIPE_COLOR: Color = Color::srgb(0.678, 0.671, 0.643);

/// Слой промзоны — своя метка: пересобирается он по солнцу и по наклону.
#[derive(Component)]
pub struct IndustryLayerTag;

/// Тон верха и стены. Металл резервуара светлый и холодный, бетон трубы
/// тёмный и тёплый — на снимке это два разных материала, и на общем плане
/// только по тону их и различить.
struct StructureLook {
    top: Color,
    wall: Color,
}

fn look_of(kind: StructureKind) -> StructureLook {
    let (top, wall) = match kind {
        StructureKind::Tank => ((0.722, 0.722, 0.702), (0.560, 0.560, 0.549)),
        StructureKind::Silo => ((0.741, 0.729, 0.702), (0.580, 0.569, 0.549)),
        StructureKind::Gasometer => ((0.678, 0.678, 0.667), (0.518, 0.518, 0.510)),
        StructureKind::Chimney => ((0.600, 0.560, 0.518), (0.459, 0.427, 0.396)),
        StructureKind::WaterTower => ((0.702, 0.678, 0.651), (0.541, 0.522, 0.502)),
    };
    StructureLook {
        top: Color::srgb(top.0, top.1, top.2),
        wall: Color::srgb(wall.0, wall.1, wall.2),
    }
}

pub fn rebuild_industry(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    map: Res<MapData>,
    mode: Res<BuildingHeightMode>,
    existing: Query<Entity, With<IndustryLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if map.structures.is_empty() && map.pipes.is_empty() {
        return;
    }

    let mut pipe_shadows = MeshBuilder::default();
    let mut pipes = MeshBuilder::default();
    let offset = shadow_dir() * (PIPE_HEIGHT * shadow_length_scale());
    for pipe in &map.pipes {
        let shifted: Vec<Vec2> = pipe.points.iter().map(|point| *point + offset).collect();
        push_pipe(&mut pipe_shadows, &shifted, pipe.width, SHADOW_COLOR);
    }
    for pipe in &map.pipes {
        push_pipe(&mut pipes, &pipe.points, pipe.width, PIPE_COLOR);
    }

    let mut shadows = MeshBuilder::default();
    let mut walls = MeshBuilder::default();
    let mut tops = MeshBuilder::default();
    for structure in &map.structures {
        if mode.casts_shadows() {
            push_shadow(&mut shadows, structure);
        }
        let lift = drawn_lift(structure.height, *mode);
        push_wall(&mut walls, structure, lift);
        push_top(&mut tops, structure, lift);
    }
    let vertices = pipe_shadows.vertex_count()
        + pipes.vertex_count()
        + shadows.vertex_count()
        + walls.vertex_count()
        + tops.vertex_count();

    // тень полупрозрачна, верх и стена нет — блендинг нужен только первому
    let shadow_material = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    let flat = materials.add(Color::WHITE);
    for (builder, z, name, material) in [
        (
            pipe_shadows,
            Z_PIPE_SHADOW,
            "pipe_shadows",
            shadow_material.clone(),
        ),
        (pipes, Z_PIPE, "pipes", flat.clone()),
        (
            shadows,
            Z_INDUSTRY_SHADOW,
            "industry_shadows",
            shadow_material,
        ),
        (walls, Z_INDUSTRY_WALL, "industry_walls", flat.clone()),
        (tops, Z_INDUSTRY, "industry_tops", flat),
    ] {
        surface::spawn_layer(
            &mut commands,
            &mut meshes,
            builder,
            z,
            name,
            LayerMaterial::Flat(material),
            IndustryLayerTag,
        );
    }
    info!(
        "industry: {} structures, {} pipes ({vertices} verts)",
        map.structures.len(),
        map.pipes.len()
    );
}

/// Труба лентой: та же лента, что у ограды, только шире и со скруглённым
/// изломом — теплотрасса поворачивает отводом, а не углом.
fn push_pipe(builder: &mut MeshBuilder, points: &[Vec2], width: f32, color: Color) {
    builder.push_ribbon(
        points,
        false,
        width,
        color.to_linear(),
        RibbonJoin::Round,
        RibbonCap::Butt,
    );
}

/// Тень цилиндра — свип его круга по свету: цилиндр сплошной от земли до
/// верха, поэтому тень его силуэта это не сдвинутый круг, а оболочка круга и
/// сдвинутого круга разом. Тем же свипом рисуются тени домов, только там его
/// считает `i_overlay` по контуру, а у круга он выписывается руками.
///
/// Зажим длины — тот же `SHADOW_LENGTH_RANGE`, что у домов, и, как у них,
/// растянутый по высоте солнца ([`sun_stretch`]): числа подобраны под
/// `cot 59°`, и неподвижный потолок в сорок пять метров на низком солнце
/// уравнял бы тень трубы с тенью пятиэтажки.
fn push_shadow(builder: &mut MeshBuilder, structure: &Structure) {
    let stretch = sun_stretch();
    let length = (structure.height * shadow_length_scale()).clamp(
        *SHADOW_LENGTH_RANGE.start() * stretch,
        *SHADOW_LENGTH_RANGE.end() * stretch,
    );
    let offset = shadow_dir() * length;
    builder.push_polygon(
        &sweep(structure.at, structure.radius, offset),
        &[],
        SHADOW_COLOR.to_linear(),
    );
}

/// Стена: по квадý на грань, от основания до поднятого верха.
///
/// **Видна половина, обращённая против крена** — та, что смотрит на камеру:
/// верх уезжает от неё, и на снимке у накренённого цилиндра виден его ближний
/// бок. Дальняя половина спрятана за ним целиком, и рисовать её незачем.
///
/// Дна при этом не видно **никогда**, и это не вопрос вкуса, а геометрия:
/// силуэт накренённого цилиндра — стадион (оболочка круга основания и круга
/// верха), круг верха закрывает его верхушку, и ровно остаток закрывают квады
/// ближней половины. Проверяется в одну строчку: при `p` на ближней дуге
/// точка `p + t·lift` пробегает по вертикали `[p_y, p_y + |lift|]`, а верх —
/// `[|lift| − r, |lift| + r]`, и вместе это весь стадион `[−r, |lift| + r]`.
///
/// Пока рисовалась дальняя половина, ближний торец оставался дырой, в которую
/// смотрела тень (тень цилиндра включает круг его основания: земля под ним
/// закрыта от неба целиком) — и из-под трубы вылезал сначала тёмный полудиск,
/// а после попытки закрыть его подошвой — само дно.
fn push_wall(builder: &mut MeshBuilder, structure: &Structure, lift: Vec2) {
    let Some(toward) = lift.try_normalize() else {
        // без крена (плоские режимы) цилиндр стоит отвесно, стены не видно вовсе
        return;
    };
    let base = look_of(structure.kind).wall.to_srgba();
    let step = TAU / CIRCLE_SIDES as f32;
    for side in 0..CIRCLE_SIDES {
        let (from, to) = (
            Vec2::from_angle(side as f32 * step),
            Vec2::from_angle((side + 1) as f32 * step),
        );
        let outward = (from + to).normalize_or(from);
        if outward.dot(toward) > 0.0 {
            continue;
        }
        let (left, right) = (
            structure.at + from * structure.radius,
            structure.at + to * structure.radius,
        );
        builder.push_quad(
            [left, right, right + lift, left + lift],
            shade_by_light(base, outward, WALL_LIT_MIX, WALL_SHADED_MIX).into(),
        );
    }
}

/// Верх: круг и тёмная кромка по краю.
fn push_top(builder: &mut MeshBuilder, structure: &Structure, lift: Vec2) {
    let look = look_of(structure.kind);
    let at = structure.at + lift;
    builder.push_polygon(&disc(at, structure.radius), &[], look.top.to_linear());

    let width = (structure.radius * RIM_SHARE).clamp(*RIM_RANGE.start(), *RIM_RANGE.end());
    if structure.radius <= width {
        return;
    }
    let rim: LinearRgba = look.top.to_srgba().mix(&Srgba::BLACK, RIM_MIX).into();
    let step = TAU / CIRCLE_SIDES as f32;
    let (inner, outer) = (structure.radius - width, structure.radius);
    for side in 0..CIRCLE_SIDES {
        let (from, to) = (
            Vec2::from_angle(side as f32 * step),
            Vec2::from_angle((side + 1) as f32 * step),
        );
        builder.push_quad(
            [
                at + from * inner,
                at + to * inner,
                at + to * outer,
                at + from * outer,
            ],
            rim,
        );
    }
}

/// Круг правильным многоугольником.
fn disc(at: Vec2, radius: f32) -> Vec<Vec2> {
    let step = TAU / CIRCLE_SIDES as f32;
    (0..CIRCLE_SIDES)
        .map(|side| at + Vec2::from_angle(side as f32 * step) * radius)
        .collect()
}

/// Выпуклая оболочка круга и его копии, сдвинутой на `offset`, — «стадион».
/// Дальняя половина дуги сдвинута, ближняя нет, и прямые борта между ними
/// получаются сами.
fn sweep(at: Vec2, radius: f32, offset: Vec2) -> Vec<Vec2> {
    let Some(along) = offset.try_normalize() else {
        return disc(at, radius);
    };
    let half = CIRCLE_SIDES / 2;
    let step = PI / half as f32;
    let point = |angle: f32| along.rotate(Vec2::from_angle(angle)) * radius;
    let mut ring = Vec::with_capacity(CIRCLE_SIDES + 2);
    // дальняя дуга, от «правой» нормали через сторону сдвига к «левой»
    for index in 0..=half {
        ring.push(at + offset + point(-FRAC_PI_2 + step * index as f32));
    }
    // и обратно ближней дугой, уже без сдвига
    for index in 0..=half {
        ring.push(at + point(FRAC_PI_2 + step * index as f32));
    }
    ring
}

#[cfg(test)]
mod tests;
