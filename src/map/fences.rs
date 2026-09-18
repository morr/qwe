//! Заборы: линия и её тень.
//!
//! В частном секторе с воздуха видны не дома, а **участки**: земля разрезана
//! сеткой заборов, и без неё дома стоят в общем поле, чего в жизни не бывает.
//! В Туле таких линий 429: 356 заборов, 72 стены (одна из них подпорная —
//! `retaining_wall` рисуется стеной) и одна живая изгородь.
//!
//! Забор сверху — это волосок в четверть метра, и держится он на снимке
//! **тенью**: собственная линия почти не видна, а тень от неё вытекает
//! из-под неё тёмной полосой. Поэтому рисуются обе, и тень — первой.
//!
//! Ширина линии растёт с отдалением ([`FENCE_LODS`], приём трамвая): четверть
//! метра — это меньше пикселя уже на 0.3 м/px, то есть ровно там, где забор
//! обязан быть виден. Целиться в ~1.5 экранных пикселя честнее, чем рисовать
//! честные 25 см и не показать ничего.
//!
//! **Нарисованная ограда и непроходимая — одна и та же.** В навмеше ограда
//! лежит со своей физической толщиной (`footprint::FENCE_BAND_WIDTH`), а не с
//! шириной ленты на экране, и с проёмами там, где сквозь неё идёт дорога или
//! открыта калитка по умолчанию (`footprint::fence_gaps`,
//! `Navmesh::open_sealed_fences`). Здесь в тех же проёмах нет ни линии, ни
//! тени (`footprint::fence_pieces`): сплошной забор поперёк тропинки, по
//! которой идут пешки, врал бы о проходимости.

use bevy::prelude::*;

use crate::map::SunOnMap;
use crate::map::footprint::{fence_gaps, fence_pieces};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin, sweep_convex};
use crate::map::osm::{FenceKind, FenceLine, MapData, RoadLine};
use crate::map::shadow;
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use crate::settings::Z_FENCE;

/// Высота забора, м: по ней считается длина тени тем же котангенсом высоты
/// солнца, что у домов и вагонов. Двухметровый глухой забор частного сектора.
const FENCE_HEIGHT: f32 = 2.0;
/// Живая изгородь ниже и мягче.
const HEDGE_HEIGHT: f32 = 1.4;

/// Ступень зум-LOD: ширина линии, м. Считана из ~1.5 экранных пикселей на
/// **худшем** краю ступени — той же меркой, что ширина трамвайной линии.
pub struct FenceLod {
    pub max_zoom: f32,
    pub width: f32,
}

pub const FENCE_LODS: [FenceLod; 4] = [
    // вблизи — настоящая доска
    FenceLod {
        max_zoom: 0.12,
        width: 0.25,
    },
    FenceLod {
        max_zoom: 0.35,
        width: 0.5,
    },
    FenceLod {
        max_zoom: 0.9,
        width: 1.3,
    },
    // дальше забор не рисуется вовсе: на общем плане сетка участков
    // превращается в грязь
    FenceLod {
        max_zoom: f32::INFINITY,
        width: 0.0,
    },
];

pub enum FenceLods {}

impl ZoomLods for FenceLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        FENCE_LODS.iter().map(|lod| lod.max_zoom)
    }
}

pub type FenceZoomBucket = ZoomBucket<FenceLods>;

/// Слой заборов — своя метка: пересобирается он по ступени зума и по солнцу.
///
/// `Copy` — потому что метку получает каждый слой модуля, а метка пуста.
#[derive(Component, Clone, Copy)]
pub struct FenceLayerTag;

/// Цвета: доска и профнастил серо-бурые, бетонная стена светлее и холоднее,
/// изгородь зелёная.
const FENCE_COLOR: Color = Color::srgb(0.435, 0.404, 0.353);
const WALL_COLOR: Color = Color::srgb(0.549, 0.541, 0.522);
const HEDGE_COLOR: Color = Color::srgb(0.298, 0.376, 0.243);

/// Когда пересобирать слой оград: своя ступень зума и осевшее солнце.
///
/// Солнце — потому что видно у ограды в первую очередь **тень**: сверху сам
/// забор это волос в четверть метра. Осевшее (`SunOnMap`), а не ползунок
/// (`SunStyle`): иначе слой пересобирался бы на каждом делении шкалы и с ещё
/// не доехавшим солнцем.
///
/// **Условие одно, регистрация одна** (см. `roads::rebuilds_on`).
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<FenceZoomBucket>.or_else(retuned::<SunOnMap>)
}

pub fn rebuild_fences(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<FenceZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<FenceLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let (layers, report) = mesh_fences(*bucket, &map.fences, &map.roads);
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials,
        layers,
        FenceLayerTag,
    );
    info!("{report}");
}

/// Что вышло из сборки заборов — значением, а не только строкой в логе.
///
/// Числа, которыми этот слой тюнился (429 линий в Туле, ширина ступени), так
/// становятся тем, что можно утверждать в тесте: `info!` на macOS ещё и меряет
/// не то время, потому что App Nap решает, как быстро идёт сборка.
///
/// Снятый слой — это состояние отчёта, а не ноль в счётчике, и состояние это
/// несёт сама `width`: нулевая ширина и значит «на этой ступени не рисуется»,
/// ровно как `None` в `CarReport::detail`. Заводить рядом с ней `hidden: bool`
/// значило бы держать два источника одного факта.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FenceReport {
    /// Сколько оград пришло на вход — считается и на снятой ступени.
    pub lines: usize,
    /// Сколько кусков осталось после вырезания проёмов под дороги и калитки:
    /// больше, чем линий, — значит проёмы разрезали ограды. На снятой ступени
    /// ноль: проёмы там не режутся вовсе (`fence_gaps` — 8.7 мс на Туле).
    pub pieces: usize,
    /// Ширина линии на этой ступени, м. Ноль — дальняя ступень, слой снят, и
    /// `Display` печатает `fences: hidden`.
    pub width: f32,
    pub vertices: usize,
}

impl std::fmt::Display for FenceReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            lines,
            pieces,
            width,
            vertices,
        } = self;
        if *width <= 0.0 {
            return write!(f, "fences: hidden ({lines} lines)");
        }
        write!(
            f,
            "fences: {lines} lines in {pieces} pieces at {width:.2} m ({vertices} verts)"
        )
    }
}

/// Сначала все тени, потом все линии: иначе тень одного забора легла бы на
/// соседний — то же правило, что у машин и вагонов.
///
/// Рисуется ограда **с проёмами**: там, где сквозь неё идёт дорога или открыта
/// калитка по умолчанию, нет ни линии, ни тени — это те же проёмы, через
/// которые ходят пешки (`footprint::fence_gaps` / `fence_pieces`), и
/// нарисованный сплошной забор поперёк тропинки врал бы о проходимости.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_fences`]), и тест. Ступень зума
/// разворачивается в ширину здесь, а не у вызывающего — как у путей, трамвая,
/// вагонов и машин; дальняя ступень (нулевая ширина) отдаёт пустой список
/// слоёв, а не особый случай у вызывающего.
pub fn mesh_fences(
    bucket: FenceZoomBucket,
    fences: &[FenceLine],
    roads: &[RoadLine],
) -> (Vec<LayerMesh>, FenceReport) {
    let width = FENCE_LODS[bucket.index].width;
    if width <= 0.0 {
        return (
            Vec::new(),
            FenceReport {
                lines: fences.len(),
                pieces: 0,
                width,
                vertices: 0,
            },
        );
    }
    let gaps = fence_gaps(fences, roads);
    let pieces: Vec<(FenceKind, Vec<Vec2>)> = fences
        .iter()
        .zip(&gaps)
        .flat_map(|(fence, gaps)| {
            fence_pieces(fence, gaps)
                .into_iter()
                .map(|piece| (fence.kind, piece))
        })
        .collect();
    let mut builder = MeshBuilder::default();
    push_shadows(&mut builder, &pieces, width);
    for (kind, points) in &pieces {
        let color = match kind {
            FenceKind::Fence => FENCE_COLOR,
            FenceKind::Wall => WALL_COLOR,
            FenceKind::Hedge => HEDGE_COLOR,
        };
        builder.push_ribbon(
            points,
            false,
            width,
            color.to_linear(),
            RibbonJoin::Round,
            RibbonCap::Round,
        );
    }
    let report = FenceReport {
        lines: fences.len(),
        pieces: pieces.len(),
        width,
        vertices: builder.vertex_count(),
    };
    // тень полупрозрачна, сама линия нет — один меш с блендингом
    let layer = LayerMesh::new(builder, Z_FENCE, "fences", MaterialSpec::Blend);
    (vec![layer], report)
}

/// Мягкий край тени, м — машинный (`cars/body.rs::SHADOW_BLUR`): тень забора
/// того же порядка длины, что у машины, а не у дома с его метровой каймой.
const SHADOW_BLUR: f32 = 0.35;

/// Сторон у многоугольника, которым приближён круглый стык и торец ленты.
const JOINT_SIDES: usize = 8;

/// Тень всех оград — одной объединённой фигурой с мягким краем.
///
/// **Свип, а не сдвиг.** Забор — стенка высотой `h` и толщиной в ленту, и
/// тень от неё — сумма Минковского ленты с отрезком света `[0, offset]`: она
/// начинается **под** забором и вытекает из-под него. Сдвинутая копия ленты,
/// что стояла здесь, при солнце 15° уезжала от двухметрового забора на 7.4 м
/// — при ширине ленты в четверть метра это вторая ограда рядом, а не тень.
/// Лента не выпукла, поэтому свип собирается по кускам, каждый из которых
/// выпуклый: прямоугольник каждого звена и многоугольник каждого узла (круглые
/// стыки и торцы ленты, `RibbonJoin::Round` / `RibbonCap::Round`), и каждый заметается
/// [`sweep_convex`] — тем же обходом, что у машин.
///
/// **Куски объединяются**, в отличие от машин: полупрозрачный слой, в котором
/// тень звена и тень узла накладываются, темнел бы пятном на каждом изломе, а
/// в частном секторе заборы стоят вплотную по границам участков, и при низком
/// солнце тени соседних оград ложатся друг на друга по всей длине. Объединение
/// (`i_overlay`, NonZero) снимает и то и другое разом — приём теней зданий.
///
/// Кайма сужается к забору по правилу зданий и машин: доля ширины на вершине
/// — проекция её направления на свет, у основания ноль.
fn push_shadows(builder: &mut MeshBuilder, pieces: &[(FenceKind, Vec<Vec2>)], width: f32) {
    let half = width / 2.0;
    let joint: Vec<Vec2> = (0..JOINT_SIDES)
        .map(|side| {
            let angle = side as f32 * std::f32::consts::TAU / JOINT_SIDES as f32;
            Vec2::from_angle(angle) * half
        })
        .collect();
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut push = |outline: &[Vec2], offset: Vec2| {
        contours.push(
            sweep_convex(outline, offset)
                .iter()
                .map(Vec2::to_array)
                .collect(),
        );
    };
    for (kind, points) in pieces {
        let height = match kind {
            FenceKind::Fence | FenceKind::Wall => FENCE_HEIGHT,
            FenceKind::Hedge => HEDGE_HEIGHT,
        };
        let offset = shadow::offset(height);
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let Some(along) = (b - a).try_normalize() else {
                continue;
            };
            let across = along.perp() * half;
            // против часовой: `perp` поворачивает направление звена влево
            push(&[a - across, b - across, b + across, a + across], offset);
        }
        for &point in points {
            let polygon: Vec<Vec2> = joint.iter().map(|&corner| point + corner).collect();
            push(&polygon, offset);
        }
    }

    shadow::push_union(builder, &contours, SHADOW_BLUR);
}

#[cfg(test)]
mod tests;
