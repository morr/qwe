//! Слитая дверь меша ([`mesh_roads`] — игру, витрины и тесты) и сырой путь
//! OSM ([`mesh_raw_roads`]), на который сворачивается сборка при
//! `RawOsm::Draw`.

use super::build::{RoadBuild, build_roads};
use super::ribbons::push_ribbon;
use super::shape::{ROAD_JOIN, RoadShape, RoadStyle};
use super::verges::VergeZoomBucket;
use super::{ALLEY_COLOR, ROAD_COLOR, RoadReport, SIDEWALK_COLOR, UNPAVED_ROAD_COLOR, WALL_COLOR};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::model::RoadAreaKind;
use crate::map::osm::{MapData, RoadClass};
use crate::map::shapes::is_ring;
use crate::map::surface::{LayerMesh, MaterialSpec, SurfaceKind};
use crate::map::z::{Z_ALLEY, Z_ROAD, Z_SIDEWALK, Z_UNPAVED_ROAD, Z_WALL};

/// Двадцать пять дорожных слоёв в выбранном стиле и форме: газон островов
/// колец, два газона широких обочин с плиткой у бордюра, замапленная трава
/// острова, заливка аллей, тротуары, газон разделительных, грунтовые улицы,
/// заливка улиц, два слоя большой стоянки, лента крепостной стены, настилы
/// переездов, три мостовых слоя и восемь слоёв краски
/// (`roads/paint.rs`) над своим асфальтом.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_roads`] и `spawn_map`), и тест. Это самый
/// крупный модуль шва, и он же самый показательный: двадцать пять слоёв,
/// четыре вида материала и вся телеметрия области — всё уезжает через один
/// возврат.
/// `verge` — ступень зума обочин ([`VergeLods`]): по правилу шва ступень, а
/// не развёрнутое из таблицы значение.
pub fn mesh_roads(
    map: &MapData,
    style: RoadStyle,
    shape: RoadShape,
    verge: VergeZoomBucket,
) -> (Vec<LayerMesh>, RoadReport) {
    let RoadBuild {
        mut layers,
        paint,
        report,
        ..
    } = build_roads(map, style, shape, verge);
    // краска — сразу над своим асфальтом: улиц над улицами, мостов над
    // настилом; сортировка устойчива, прочие слои порядка не меняют
    layers.extend(paint.into_iter().map(|layer| layer.mesh));
    layers.sort_by(|a, b| a.z.total_cmp(&b.z));
    (layers, report)
}

/// Дорожные слои **сырого OSM** (`RawOsm::Draw`): каждый way — простая лента
/// по своей оси OSM шириной, какую ему дал разбор (по тегу `lanes`, иначе по
/// классу), с прямыми торцами и стыками без скруглений. Ни склейки узлов, ни
/// пар половин, ни разделительных, ни устьев и скруглений бордюра, ни
/// тротуаров-полос, ни газонов обочин, ни стоянок-«больших лотов», ни
/// разметки и колеи — отдельный путь сборки, а не выключатели внутри
/// [`mesh_roads`]: его проходы переплетены, и «выключить половину» дало бы
/// картинку, которой нет ни в данных, ни в игре.
///
/// Что остаётся: слои по покрытию (асфальт улиц, грунтовые улицы, дорожки —
/// песком, мощёные — плиткой тротуара), площади дорог из данных своими
/// контурами (`area:highway`, `highway` + `area=yes`) и лента крепостной
/// стены. Узкие под широкими, как в игре. Мост — в том же слое, что улица:
/// настила, бордюра и тени у него нет.
pub(super) fn mesh_raw_roads(map: &MapData, style: RoadStyle) -> (Vec<LayerMesh>, RoadReport) {
    let started = std::time::Instant::now();
    let mut streets = MeshBuilder::with_surface_coords();
    let mut unpaved = MeshBuilder::with_surface_coords();
    let mut alleys = MeshBuilder::with_surface_coords();
    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut wall_ribbons = MeshBuilder::default();

    for area in &map.road_areas {
        let (builder, color) = match area.kind {
            RoadAreaKind::Carriageway => (&mut streets, ROAD_COLOR),
            RoadAreaKind::Walkway => (&mut sidewalks, SIDEWALK_COLOR),
            RoadAreaKind::Island => (&mut sidewalks, SIDEWALK_COLOR),
        };
        builder.push_polygon(&area.outline, &[], color.to_linear());
    }
    let mut order: Vec<usize> = (0..map.roads.len()).collect();
    order.sort_by(|&a, &b| map.roads[a].width.total_cmp(&map.roads[b].width));
    for index in order {
        let road = &map.roads[index];
        let (builder, color) = match road.class {
            RoadClass::Street if road.is_unpaved_street() => (&mut unpaved, UNPAVED_ROAD_COLOR),
            RoadClass::Street => (&mut streets, ROAD_COLOR),
            RoadClass::Alley if road.is_paved_path() => (&mut sidewalks, SIDEWALK_COLOR),
            RoadClass::Alley => (&mut alleys, ALLEY_COLOR),
        };
        builder.push_ribbon(
            &road.points,
            is_ring(&road.points),
            road.width,
            color.to_linear(),
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }
    for wall in &map.walls {
        push_ribbon(
            &mut wall_ribbons,
            &wall.points,
            wall.width,
            WALL_COLOR.to_linear(),
            ROAD_JOIN,
        );
    }
    let layers: Vec<LayerMesh> = [
        (
            alleys,
            Z_ALLEY,
            "alleys",
            MaterialSpec::Surface(SurfaceKind::Alley),
        ),
        (
            sidewalks,
            Z_SIDEWALK,
            "sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            unpaved,
            Z_UNPAVED_ROAD,
            "unpaved_roads",
            MaterialSpec::Surface(SurfaceKind::Unpaved),
        ),
        (
            streets,
            Z_ROAD,
            "roads",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (wall_ribbons, Z_WALL, "walls", MaterialSpec::Flat),
    ]
    .into_iter()
    .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
    .collect();
    let report = RoadReport {
        style,
        vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
        elapsed: started.elapsed(),
        ..Default::default()
    };
    (layers, report)
}
