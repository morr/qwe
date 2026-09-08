//! Железнодорожный путь — не картографический символ, а сама конструкция:
//! балласт с плечом, шпалы поперёк него и две стальные нитки по колее. Раньше
//! путь рисовался как в osm-carto (тёмная лента и белая штриховка поверх) и на
//! приближении оставался знаком на карте, тогда как всё вокруг — дома с
//! тенями, кроны, бордюры мостов — рисуется предметно.
//!
//! Предметность стоит геометрии: шпала каждые 65 см по всем путям города
//! осмысленна только там, где шпалу видно. Поэтому путь, как и трамвай
//! (`map/tram.rs`), пересобирается по ступеням зума ([`RAIL_LODS`]), и ступени
//! меняют не размер одного и того же рисунка, а сам рисунок:
//!
//! - вблизи — настоящий путь: шпалы с шагом 65 см и нитки по колее 1.5 м;
//! - на среднем плане нитки сходятся в одну полосу и убираются, шпалы редеют
//!   до штриховки поперёк балласта;
//! - на общем плане возвращается знак osm-carto — белый пунктир по тёмной
//!   ленте, потому что серая полоса без него читается как ещё одна улица.
//!
//! Слои — три меша (`Z_RAIL` / `Z_RAIL_TIE` / `Z_RAIL_STEEL`), а не один: в
//! общем меше компланарная геометрия z-файтит, а шпала одного пути обязана
//! лежать поверх балласта соседнего — иначе развязка расслаивается на
//! отдельные ways.
//!
//! Стиль дорог (`RoadStyle`) пути не касается — как и у трамвая: на ступенях
//! LOD ширины считаются от зума, и ручка сглаживания, меняющая осевую, гоняла
//! бы путь относительно неподвижного балласта. Навмеша путь не касается тоже —
//! люди ходят через рельсы как по земле.

use std::borrow::Cow;

use bevy::prelude::*;

use crate::loading::AppState;
use crate::map::meshing::{MeshBuilder, RibbonJoin};
use crate::map::osm::{MapData, RailKind, RailLine};
use crate::map::roads::{RoadJoin, RoadSmoothing, push_ribbon, smooth_path};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::settings::{Z_RAIL, Z_RAIL_STEEL, Z_RAIL_TIE};

/// Цвета одного вида пути. Действующий путь — щебень, креозотная шпала и
/// накатанная до блеска головка рельса; заброшенный — тот же путь, заросший:
/// балласт уходит в травяной оттенок, шпала седеет, нитка ржавеет.
pub struct RailPalette {
    /// Плечо балластной призмы — откос по краю, темнее верха.
    pub shoulder: Color,
    pub ballast: Color,
    pub tie: Color,
    pub steel: Color,
    /// Штриховка дальних ступеней — знак osm-carto поверх балласта.
    pub dash: Color,
}

const ACTIVE: RailPalette = RailPalette {
    shoulder: Color::srgb(0.376, 0.357, 0.325),
    ballast: Color::srgb(0.478, 0.455, 0.427),
    tie: Color::srgb(0.243, 0.196, 0.157),
    steel: Color::srgb(0.792, 0.804, 0.827),
    dash: Color::srgb(1.0, 1.0, 1.0),
};

const DISUSED: RailPalette = RailPalette {
    shoulder: Color::srgb(0.451, 0.451, 0.400),
    ballast: Color::srgb(0.549, 0.545, 0.482),
    tie: Color::srgb(0.400, 0.361, 0.302),
    steel: Color::srgb(0.545, 0.400, 0.322),
    dash: Color::srgb(0.867, 0.867, 0.867),
};

/// Ширина плеча как доля ширины балласта. Призма шире своего верха: у
/// однопутного участка верх ~4.5 м при подошве около 6 м, и именно откос
/// отделяет путь от земли, по которой он идёт.
const SHOULDER_SCALE: f32 = 1.22;

/// Ширина, зажимающая срез Chaikin у пути. Как у трамвая — константа, а не
/// ширина ступени: осевая обязана остаться одной и той же на всех ступенях,
/// иначе путь ёрзает относительно самого себя при переходе через порог зума.
const RAIL_SMOOTH_WIDTH: f32 = 5.0;

/// Стиль зафиксирован, без ручек панели (см. модульную прозу): осевая слегка
/// сглажена, стыки круглые — ломаная OSM на повороте даёт балласту заметный
/// угол, а шпалы на нём разъезжаются веером.
const RAIL_JOIN: RoadJoin = RoadJoin::Round;
const RAIL_SMOOTHING: RoadSmoothing = RoadSmoothing::Light;

/// Шпалы одной ступени: длина поперёк пути как доля ширины балласта (у
/// узкоколейки балласт уже, и шпала обязана быть короче), толщина и шаг, м.
pub struct RailTieLod {
    pub length_scale: f32,
    pub thickness: f32,
    pub spacing: f32,
}

/// Нитки одной ступени: колея и ширина самой нитки, м. Колея — абсолютная,
/// одна на все виды пути (1.5 м — стандартные 1520 мм; light_rail, метро и
/// заброшенный путь физически той же колеи, у него лишь заросший балласт), а
/// не доля балласта: на 4- и 3.5-метровом балласте доля 0.30 сводила нитки на
/// ступени 1 до 3.6 и 3.0 px, ниже порога, на котором они читаются порознь.
pub struct RailSteelLod {
    pub gauge: f32,
    pub width: f32,
}

/// Штриховка дальних ступеней: длина штриха, пропуска и ширина как доля
/// балласта — знак osm-carto, которым путь рисовался целиком до LOD.
pub struct RailDashLod {
    pub length: f32,
    pub gap: f32,
    pub width_scale: f32,
}

/// Ступень зум-LOD пути: до какого зума действует и чем на ней путь нарисован.
/// Зум — мировых метров на логический пиксель (`PanCamera`).
pub struct RailLod {
    /// Верхняя (исключающая) граница ступени.
    pub max_zoom: f32,
    /// Нижняя граница ширины балласта, м: ширина из OSM (5 м у магистрального
    /// пути, 4 у узкоколейки и метро) на общем плане города уходит в
    /// пиксель-другой, и путь пропадает раньше улиц, которые он пересекает.
    pub min_bed: f32,
    /// `None` — на этой ступени шпал нет.
    pub tie: Option<RailTieLod>,
    /// `None` — нитки сошлись бы в одну полосу и не рисуются.
    pub steel: Option<RailSteelLod>,
    /// `None` — путь читается своей конструкцией, знак не нужен.
    pub dash: Option<RailDashLod>,
}

/// Ступени зум-LOD. Числа выведены из экранного размера на **худшем** краю
/// ступени (у верхней границы зума): шаг шпал нигде не падает ниже ~6 px, ни
/// шпала, ни нитка, ни штрих — ниже ~1 px. Отсюда и три разных рисунка: с
/// 0.26 м/px колея в 1.5 м это уже 6 px, две нитки по одному пикселю в них не
/// разделяются, а с 0.65 м/px шпалы приходится ставить реже, чем их видно как
/// шпалы, — дальше честнее знак, чем стёршаяся конструкция.
///
/// Второе число, которое держится поперёк ступеней, — **доля шпалы в шаге**,
/// около 40%: у настоящего пути это 0.26 м на 0.65, и стоит ей упасть, как
/// шпалы перестают быть текстурой и становятся редкими метками, а две белые
/// нитки перевешивают их и путь читается лестницей, а не путём.
pub const RAIL_LODS: [RailLod; 5] = [
    RailLod {
        max_zoom: 0.10,
        min_bed: 0.0,
        // настоящая геометрия: шпала 2.6 × 0.26 м с шагом 65 см
        tie: Some(RailTieLod {
            length_scale: 0.52,
            thickness: 0.26,
            spacing: 0.65,
        }),
        steel: Some(RailSteelLod {
            gauge: 1.5,
            width: 0.12,
        }),
        dash: None,
    },
    RailLod {
        max_zoom: 0.26,
        min_bed: 0.0,
        tie: Some(RailTieLod {
            length_scale: 0.52,
            thickness: 0.64,
            spacing: 1.6,
        }),
        steel: Some(RailSteelLod {
            gauge: 1.5,
            width: 0.26,
        }),
        dash: None,
    },
    RailLod {
        max_zoom: 0.65,
        min_bed: 0.0,
        tie: Some(RailTieLod {
            length_scale: 0.56,
            thickness: 1.5,
            spacing: 4.0,
        }),
        steel: None,
        dash: None,
    },
    RailLod {
        max_zoom: 1.6,
        min_bed: 5.0,
        tie: None,
        steel: None,
        dash: Some(RailDashLod {
            length: 9.0,
            gap: 9.0,
            width_scale: 0.5,
        }),
    },
    RailLod {
        max_zoom: f32::INFINITY,
        min_bed: 9.0,
        tie: None,
        steel: None,
        dash: Some(RailDashLod {
            length: 22.0,
            gap: 22.0,
            width_scale: 0.5,
        }),
    },
];

/// [`RAIL_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`). Своя, а не
/// трамвайная: у пути и порогов больше, и смысл у них другой. Пустой enum —
/// тип-маркер, значений у него не бывает.
pub enum RailLods {}

impl ZoomLods for RailLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        RAIL_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`RAIL_LODS`]; пересечение порога пересобирает рельсовые
/// слои ([`rebuild_rails`]).
pub type RailZoomBucket = ZoomBucket<RailLods>;

/// Рельсовый слой карты — чтобы пересборка знала, что деспавнить.
#[derive(Component)]
pub struct RailLayerTag;

/// Один путь, приведённый к тому, что нужно рисованию: сглаженная осевая,
/// ширина балласта на этой ступени и палитра своего вида. Считается один раз,
/// потому что проходов по путям несколько — слои обязаны собираться целиком,
/// а не путь за путём (см. модульную прозу про z-файтинг).
struct Track<'a> {
    points: Cow<'a, [Vec2]>,
    bed: f32,
    palette: &'static RailPalette,
}

/// Рельсовые слои текущей ступени зума. Единственный вызов — из
/// [`rebuild_rails`]: и вход в мир, и смена ступени идут через пересборку (в
/// свежем мире деспавнить ей нечего).
fn spawn_rails(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    bucket: RailZoomBucket,
    rails: &[RailLine],
) {
    let started = std::time::Instant::now();
    let lod = &RAIL_LODS[bucket.index];

    let tracks: Vec<Track> = rails
        .iter()
        .filter_map(|rail| {
            let palette = match rail.kind {
                // трамвай — свой меш со своим стилем и зум-LOD (`map/tram.rs`)
                RailKind::Tram => return None,
                RailKind::Active => &ACTIVE,
                RailKind::Disused => &DISUSED,
            };
            Some(Track {
                points: smooth_path(&rail.points, RAIL_SMOOTH_WIDTH, RAIL_SMOOTHING),
                bed: rail.width.max(lod.min_bed),
                palette,
            })
        })
        .collect();

    let mut ballast = MeshBuilder::default();
    for track in &tracks {
        push_ribbon(
            &mut ballast,
            &track.points,
            track.bed * SHOULDER_SCALE,
            track.palette.shoulder.to_linear(),
            RAIL_JOIN,
        );
    }
    // верх призмы — вторым проходом: плечо соседнего пути не должно ложиться
    // на балласт этого, иначе развязка расчерчивается тёмными полосами
    for track in &tracks {
        push_ribbon(
            &mut ballast,
            &track.points,
            track.bed,
            track.palette.ballast.to_linear(),
            RAIL_JOIN,
        );
    }

    let mut ties = MeshBuilder::default();
    for track in &tracks {
        if let Some(tie) = &lod.tie {
            ties.push_ticks(
                &track.points,
                track.bed * tie.length_scale,
                tie.thickness,
                tie.spacing,
                track.palette.tie.to_linear(),
            );
        }
        if let Some(dash) = &lod.dash {
            ties.push_dashes(
                &track.points,
                track.bed * dash.width_scale,
                dash.length,
                dash.gap,
                track.palette.dash.to_linear(),
                RibbonJoin::Round,
            );
        }
    }

    let mut steel = MeshBuilder::default();
    for track in &tracks {
        if let Some(rails_lod) = &lod.steel {
            steel.push_rails(
                &track.points,
                rails_lod.gauge,
                rails_lod.width,
                track.palette.steel.to_linear(),
                RibbonJoin::Round,
            );
        }
    }

    let vertices = ballast.vertex_count() + ties.vertex_count() + steel.vertex_count();
    for (builder, z, name) in [
        (ballast, Z_RAIL, "rail_ballast"),
        (ties, Z_RAIL_TIE, "rail_ties"),
        (steel, Z_RAIL_STEEL, "rail_steel"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            RailLayerTag,
            Mesh2d(meshes.add(builder.build())),
            // вершинные цвета — материал белый, как у остальных слоёв карты
            MeshMaterial2d(materials.add(Color::WHITE)),
            Transform::from_xyz(0.0, 0.0, z),
            DespawnOnExit(AppState::Playing),
            Name::new(name),
        ));
    }

    info!(
        "rail meshing: {vertices} verts in {:?} (bucket {})",
        started.elapsed(),
        bucket.index,
    );
}

/// Пересборка рельсовых слоёв при смене ступени зума — дорожные и трамвайный
/// слои не трогаются.
pub fn rebuild_rails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<RailZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<RailLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_rails(
        &mut commands,
        &mut meshes,
        &mut materials,
        *bucket,
        &map.rails,
    );
}

#[cfg(test)]
mod tests;
