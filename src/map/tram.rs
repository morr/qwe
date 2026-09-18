//! Трамвайные пути, вынесенные из дорожных слоёв: трамвай пересобирается по
//! ступеням зума ([`TRAM_LODS`]) — линия держит почти постоянную экранную
//! толщину («почти gizmo»), а шпалы редеют с отъездом камеры и на общем плане
//! исчезают, иначе они сливаются в сплошную массу. Обычные ж/д пути — в
//! `map/rail.rs`, со своей таблицей ступеней и своими слоями; стиль дорог
//! (`RoadStyle`) не касается ни тех, ни других. Ручка у трамвая одна —
//! [`TramStyle::visible`], рисовать его или нет.
//!
//! Навмеша путь не касается — люди ходят через рельсы как по земле.

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RailKind, RailLine};
use crate::map::roads::{RoadJoin, RoadSmoothing, push_ribbon, smooth_path};
use crate::map::surface::{self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::settings::Z_TRAM;

/// Трамвай — не лента, а линия с поперечной насечкой, как в Яндекс.Картах и
/// 2ГИС. Причина не в стиле: трамвайный путь лежит **на проезжей части**, и
/// лента в ширину колеи закрыла бы улицу, по которой он идёт.
///
/// Цвет — единственное, чем два этих источника различаются: у Яндекса линия
/// тёмно-красная, у 2ГИС синяя. Геометрия одна и та же; взят вариант 2ГИС —
/// синее на сером асфальте видно лучше, а красным на карте уже размечены
/// стены Кремля.
const TRAM_COLOR: Color = Color::srgb(0.290, 0.451, 0.780);

/// Ширина, зажимающая срез Chaikin у трамвая. Ширина линии меняется от ступени
/// к ступени, но осевая обязана оставаться одной и той же — иначе путь ёрзает
/// при переходе через порог зума.
const TRAM_SMOOTH_WIDTH: f32 = 1.2;

/// Стиль зафиксирован — панель переключает только видимость ([`TramStyle`]): на
/// линии в полтора-два экранных пикселя стык излома не читается вовсе, а
/// Strong-сглаживание неотличимо от Light. Осевая всегда слегка сглажена —
/// ломаная OSM на повороте даёт тонкой линии заметный угол.
const TRAM_JOIN: RoadJoin = RoadJoin::Round;
const TRAM_SMOOTHING: RoadSmoothing = RoadSmoothing::Light;

/// Единственная ручка трамвая — рисовать его или нет; строка `Tram` в секции
/// Roads (`ui/roads.rs`), пишется и по BRP, сохраняется между запусками.
/// Правка пересобирает только трамвайный слой ([`rebuild_tram`]) — держать
/// тумблер в [`RoadStyle`](crate::map::RoadStyle) значило бы гнать полную
/// пересборку дорожных слоёв на каждое переключение трамвая.
///
/// Синяя линия с насечкой лежит на самой проезжей части и на общем плане
/// читается как ещё один слой улиц — трамвай выключен, пока его не включат.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "tram")]
pub struct TramStyle {
    pub visible: bool,
}

/// Шпала одной ступени: длина поперёк пути, толщина и шаг, м. Насечка обязана
/// быть заметно длиннее толщины самой линии — иначе она сливается с ней в
/// утолщение.
pub struct TramTieLod {
    pub length: f32,
    pub thickness: f32,
    pub spacing: f32,
}

/// Ступень зум-LOD трамвая: до какого зума действует и какой геометрией
/// рисуется. Зум — мировых метров на логический пиксель (`PanCamera`).
pub struct TramLod {
    /// Верхняя (исключающая) граница ступени.
    pub max_zoom: f32,
    pub line_width: f32,
    /// `None` — на этой ступени шпалы не рисуются вовсе.
    pub tie: Option<TramTieLod>,
}

/// Ступени зум-LOD: линия целится в ~1.8 px на середине каждой ступени
/// (экранная толщина гуляет в пределах ~1.1–2.9 px — «почти gizmo»), шаг шпал
/// на экране нигде не падает ниже ~10 px — редкая насечка, а не гребёнка. На
/// последней ступени шпалы исчезают, как в 2ГИС на общем плане города.
pub const TRAM_LODS: [TramLod; 5] = [
    TramLod {
        max_zoom: 0.12,
        line_width: 0.14,
        tie: Some(TramTieLod {
            length: 0.45,
            thickness: 0.09,
            spacing: 1.45,
        }),
    },
    TramLod {
        max_zoom: 0.30,
        line_width: 0.34,
        tie: Some(TramTieLod {
            length: 1.1,
            thickness: 0.20,
            spacing: 3.5,
        }),
    },
    TramLod {
        max_zoom: 0.75,
        line_width: 0.85,
        tie: Some(TramTieLod {
            length: 2.8,
            thickness: 0.50,
            spacing: 8.8,
        }),
    },
    TramLod {
        max_zoom: 1.9,
        line_width: 2.1,
        tie: Some(TramTieLod {
            length: 7.0,
            thickness: 1.2,
            spacing: 27.0,
        }),
    },
    TramLod {
        max_zoom: f32::INFINITY,
        line_width: 5.2,
        tie: None,
    },
];

/// [`TRAM_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`). Пустой enum —
/// тип-маркер, значений у него не бывает.
pub enum TramLods {}

impl ZoomLods for TramLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        TRAM_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`TRAM_LODS`]; пересечение порога пересобирает трамвайный
/// меш ([`rebuild_tram`]).
pub type TramZoomBucket = ZoomBucket<TramLods>;

/// Трамвайный меш — чтобы пересборка знала, что деспавнить.
///
/// `Copy` — метку получает каждый слой модуля, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct TramLayerTag;

/// Что вышло из сборки трамвая — значением, а не только строкой в логе.
///
/// `tracks` — сколько трамвайных путей пришло **на вход**: обычный рельсовый
/// путь сюда не попадает, у него свой модуль. Не «сколько нарисовано»:
/// выключенный тумблер — это состояние отчёта (`hidden`), а не ноль в счётчике,
/// иначе лог-строка снятого слоя неотличима от города без трамвая. Правило
/// машин (`CarReport::detail`), одно на все пять слоёв.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TramReport {
    pub tracks: usize,
    /// Тумблер выключен: слой описан и пуст, вершин ноль.
    pub hidden: bool,
    pub bucket: usize,
    pub vertices: usize,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for TramReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            tracks,
            hidden,
            bucket,
            vertices,
            elapsed,
        } = self;
        if *hidden {
            return write!(f, "tram meshing: hidden ({tracks} tracks)");
        }
        write!(
            f,
            "tram meshing: {tracks} tracks, {vertices} verts in {elapsed:?} (bucket {bucket})"
        )
    }
}

/// Трамвайный слой текущей ступени зума.
///
/// **Чистая функция и единственная дверь в слой.** Линия и шпалы — один цвет,
/// поэтому лежат в одном меше: накладываться сами на себя они могут без всякого
/// z-файтинга.
///
/// **Выключенный трамвай — это пустой список слоёв, а не ранний выход у
/// вызывающего.** Ровно тот же приём, что нулевая ширина у заборов: деспавн в
/// адаптере безусловен, и второго пути, который мог бы его забыть, нет вовсе.
/// Пути при этом считаются всё равно — снятый слой говорит о себе
/// [`TramReport::hidden`], а не нулём в счётчике.
pub fn mesh_tram(
    bucket: TramZoomBucket,
    style: &TramStyle,
    rails: &[RailLine],
) -> (Vec<LayerMesh>, TramReport) {
    let started = std::time::Instant::now();
    let lod = &TRAM_LODS[bucket.index];

    let mut builder = MeshBuilder::default();
    let mut tracks = 0;
    for rail in rails {
        if rail.kind != RailKind::Tram {
            continue;
        }
        // счёт идёт по входу, рисование — по тумблеру: «слой снят» говорит
        // `TramReport::hidden`, а ноль в счётчике остаётся означать «трамвая на
        // карте нет»
        tracks += 1;
        if !style.visible {
            continue;
        }
        let points = smooth_path(&rail.points, TRAM_SMOOTH_WIDTH, TRAM_SMOOTHING);
        push_tram(&mut builder, &points, lod);
    }

    let report = TramReport {
        tracks,
        hidden: !style.visible,
        bucket: bucket.index,
        vertices: builder.vertex_count(),
        elapsed: started.elapsed(),
    };
    // вершинные цвета — материал белый, как у остальных слоёв карты
    let layer = LayerMesh::new(builder, Z_TRAM, "tram", MaterialSpec::Flat);
    (vec![layer], report)
}

/// Линия и шпалы одного пути на одной ступени LOD — отдельно от спавна ради
/// тестов на геометрию.
pub(crate) fn push_tram(builder: &mut MeshBuilder, points: &[Vec2], lod: &TramLod) {
    let color = TRAM_COLOR.to_linear();
    push_ribbon(builder, points, lod.line_width, color, TRAM_JOIN);
    if let Some(tie) = &lod.tie {
        builder.push_ticks(points, tie.length, tie.thickness, tie.spacing, color);
    }
}

/// Офлайн-замер трамвайного слоя — по строке на ступень зума, как у рельсов:
/// ступени здесь тоже отличаются тем, что нарисовано (на дальней ступени шпал
/// нет вовсе).
///
/// Меряется **включённый** трамвай, хотя по умолчанию он выключен: замер о
/// цене слоя, а не о том, показан ли он в игре. Своей сборки у него нет — тот
/// же [`mesh_tram`], что и у игры.
pub fn measure_tram(rails: &[RailLine]) -> Vec<(usize, Vec<LayerCost>)> {
    let style = TramStyle { visible: true };
    (0..TRAM_LODS.len())
        .map(|index| {
            let (layers, report) = mesh_tram(TramZoomBucket::at(index), &style, rails);
            (index, surface::layer_costs(&layers, report.elapsed))
        })
        .collect()
}

/// Пересборка трамвайного меша при смене ступени зума или переключении
/// [`TramStyle`] — дорожные и рельсовые слои не трогаются. Выключенный трамвай
/// проходит через ту же пересборку: деспавн старого слоя и никакого нового.
pub fn rebuild_tram(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<TramZoomBucket>,
    style: Res<TramStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<TramLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let (layers, report) = mesh_tram(*bucket, &style, &map.rails);
    spawn_layers(&mut commands, &mut meshes, &materials, layers, TramLayerTag);
    info!("{report}");
}

#[cfg(test)]
mod tests;
