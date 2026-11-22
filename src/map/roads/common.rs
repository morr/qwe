//! Общая середина сборок ступеней обочин: [`RoadCommon`] — сборщики общих
//! фаз и их конструктор (`new` — фазы прохода в порядке их следования, и
//! этот порядок — инвариант), [`StepLay`] — уклад одной ступени, и финалы:
//! [`RoadCommon::take_common_layers`], [`RoadCommon::finish_step_split`].

use bevy::prelude::*;

use super::bridges::Bridges;
use super::build::{DrawnPiece, RoadSkeleton, StepRoadBuild};
use super::drawn::Axis;
use super::islands::push_street_fill;
use super::junctions::{MOUTH_CROSSING_GAP, mouths};
use super::ribbons::{wedge_halves, wedge_middle};
use super::rings::{ApproachBand, ring_island_grass, ring_island_lawns};
use super::ruts::RutLines;
use super::tram_lay::TramTracks;
use super::walls::Meadows;
use super::{
    ROAD_COLOR, RoadReport, SIDEWALK_COLOR, VergeZoomBucket, aprons, bridges, corners, fill_color,
    fill_order, gores, is_carriageway, islands, junctions, lane_count, lots, medians, merges,
    node_paint, paint, pockets, ribbon_lane, rings, road_lanes, scraps, tapers, tram_band, verges,
    walls,
};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::parallel::in_parallel;
use crate::map::rail::{CrossedStreet, mesh_level_crossings};
use crate::map::shapes::push_shape;
use crate::map::surface::{LayerMesh, MaterialSpec, SurfaceKind};
use crate::map::z::{
    Z_ALLEY, Z_LOT_LINES, Z_LOT_SIDEWALK, Z_RAIL_CROSSING, Z_RING_GRASS, Z_RING_ISLAND, Z_ROAD,
    Z_ROAD_MEDIAN, Z_ROAD_VERGE, Z_ROAD_VERGE_LAWN, Z_ROAD_VERGE_YARD, Z_SIDEWALK, Z_UNPAVED_ROAD,
    Z_WALL,
};

/// Обочинный уклад одной ступени: её скругления и обочины — в своих
/// сборщиках, вклады в лоскуты и куски — при ней. Общие фазы кладут мимо
/// ([`RoadCommon`]); финал ступени собирает слой из её кусков и общих.
pub(super) struct StepLay {
    verge: VergeZoomBucket,
    kerb: corners::KerbReturns,
    /// Скругления узлов — асфальт, тротуары, проезды. У ступеней они
    /// бит-в-бит одинаковы (отпечаток записи 28), но у каждой свои: так
    /// порядок сетки лоскутов и кусков ступени остаётся её собственным.
    streets: MeshBuilder,
    alleys: MeshBuilder,
    unpaved: MeshBuilder,
    sidewalks: MeshBuilder,
    /// Обочины до отдельных тротуаров и их газоны — слои, где ступени
    /// расходятся.
    verges: MeshBuilder,
    verge_lawns: MeshBuilder,
    verge_yards: MeshBuilder,
    /// Вклады ступени в лоскуты: скругления, потом улицы по порядку заливки.
    /// Сетка по ним ленивая — к первому вопросу все вклады дописаны.
    scraps: scraps::Scraps,
    /// Отрисованные куски ступени — для семантического индекса.
    pieces: Vec<DrawnPiece>,
    /// Обочины по кускам улиц, в порядке кусков; раскладывает лента.
    verge_chunks: Vec<verges::StreetVerges>,
}

/// Общая середина сборок ступеней: всё, что у ступеней бит-в-бит одинаково
/// (отпечаток записи 28 — асфальт, тротуары, разделительные, краска, мосты,
/// кольца, стены, переезды), считается и укладывается один раз в свои
/// сборщики. Ступенезависимое — скругления, обочины, лоскуты — живёт в
/// [`StepLay`] на ступень. Финал ([`RoadCommon::finish_step_split`])
/// собирает слои ступени: её куски, потом общие, потом лоскуты; слитая
/// дверь склеивает из этого выхода полную картину ([`build_roads_from`]).
pub(super) struct RoadCommon<'s> {
    pub(super) lays: Vec<StepLay>,
    // ── общие сборщики: асфальт, тротуары, разделительные, краска, мосты ──
    streets: MeshBuilder,
    sidewalks: MeshBuilder,
    alleys: MeshBuilder,
    unpaved: MeshBuilder,
    median_grass: MeshBuilder,
    ring_lawns: MeshBuilder,
    ring_grass: MeshBuilder,
    wall_ribbons: MeshBuilder,
    rail_crossings: MeshBuilder,
    bridges: Bridges,
    painter: paint::Painter,
    /// Слои стоянок с островками дорог — у ступеней одинаковы (отпечаток
    /// записи 28), считаются в общей середине, финал берёт копию.
    lot_layers: lots::LotLayers,
    // ── общие значения ──
    drawn: Vec<&'s RoadLine>,
    ribbon: Vec<std::borrow::Cow<'s, [Vec2]>>,
    gores: gores::Gores,
    road_islands: islands::RoadIslands,
    ruts: RutLines,
    tram_tracks: TramTracks,
    // ── отчётные значения ──
    merge_edges: usize,
    kerb_pockets: usize,
    turning_circles: usize,
    tram_bands: usize,
    track_bridges: usize,
    level_crossings: usize,
    network_time: std::time::Duration,
    started: std::time::Instant,
}

impl<'s> RoadCommon<'s> {
    /// Общая середина сборок ступеней `steps`: скругления каждой ступени
    /// (калькуляция и укладка в её [`StepLay`]), потом общие фазы — кромки,
    /// карманы, устья, разделительные, островки, ленты, краска узлов, трамвай,
    /// переезды, стены. Читает ступень `[0]` там, где общий выход от ступени
    /// не зависит (отпечаток записи 28): у одноступенчатой сборки это её
    /// собственная ступень.
    pub(super) fn new(skeleton: &'s RoadSkeleton<'s>, steps: &[VergeZoomBucket]) -> RoadCommon<'s> {
        let RoadSkeleton {
            map,
            style,
            shape,
            prepared,
            stations,
            islands,
            junctions,
            turns,
            splitters,
        } = skeleton;
        let map = *map;
        let style = *style;
        let shape = *shape;
        // ── Слои и накопители ── куда кладут общие фазы ниже: строители
        // слоёв, мосты, стоянки, краска и колея
        let started = std::time::Instant::now();
        let roads: &[RoadLine] = &map.roads;
        let mut painter = paint::Painter::new(map.traffic_side);
        let mut ruts = RutLines::default();

        let mut sidewalks = MeshBuilder::with_surface_coords();
        let mut median_grass = MeshBuilder::with_surface_coords();
        let mut alleys = MeshBuilder::with_surface_coords();
        let mut streets = MeshBuilder::with_surface_coords();
        // грунтовые улицы — своим слоем под асфальтом (`Z_UNPAVED_ROAD`)
        let mut unpaved = MeshBuilder::with_surface_coords();
        // мост — цепочка ways, и тень считается по всей цепочке; мостовые слои
        // копит он же (`roads/bridges.rs`)
        let mut bridges = Bridges::new(map);
        let mut wall_ribbons = MeshBuilder::default();
        // улицы на больших стоянках — бордюром и разметкой поверх их асфальта
        // (`roads/lots.rs`)
        let mut grounds = lots::Grounds::of(map);

        // ── Нарисованная сеть ──
        // Скелет сети — подготовка, станции, островки узлов, узлы с краской,
        // траектории, островки подходов — собран в скелете (`road_skeleton`):
        // у ступеней он общий. Общие фазы идут под ним.
        let nodes = prepared.nodes();
        let drawn = prepared.roads();
        // узловые оси — скругления, слияния, карманы, станции штрихов, полотно
        // трамвая: их торцы стоят в точках OSM
        let nodal = prepared.axes(Axis::Nodal);
        // ── Скругления ступеней ── калькуляция и укладка — в уклад ступени,
        // ступень от ступени не зависит — по ядрам: у ступеней свои сетки
        // лоскутов и свои куски. Скругления кладутся раньше всех лент своего
        // слоя: лента поверх кроет скругление, а не наоборот, и разметка
        // остаётся целой
        let meadows = Meadows::of(&map.parks, &map.grass);
        let mut lays: Vec<StepLay> = in_parallel(steps, |&verge| {
            let kerb = corners::kerb_returns(prepared, shape.corner_radius(), verge);
            let mut lay = StepLay {
                verge,
                kerb,
                streets: MeshBuilder::with_surface_coords(),
                alleys: MeshBuilder::with_surface_coords(),
                unpaved: MeshBuilder::with_surface_coords(),
                sidewalks: MeshBuilder::with_surface_coords(),
                verges: MeshBuilder::with_surface_coords(),
                verge_lawns: MeshBuilder::with_surface_coords(),
                verge_yards: MeshBuilder::with_surface_coords(),
                scraps: scraps::Scraps::default(),
                pieces: Vec::new(),
                verge_chunks: Vec::new(),
            };
            // асфальт узлов — в свои слои; порядок укладки — инвариант
            // `corners` (`KerbReturns::lay`), а не этого вызывающего
            lay.kerb.lay(
                corners::KerbLayers {
                    streets: &mut lay.streets,
                    alleys: &mut lay.alleys,
                    unpaved: &mut lay.unpaved,
                    sidewalks: &mut lay.sidewalks,
                    verges: &mut lay.verges,
                    lawns: &mut lay.verge_lawns,
                    yards: &mut lay.verge_yards,
                },
                &meadows,
                &mut lay.scraps,
                &mut lay.pieces,
            );
            lay
        });
        // общие фазы читают скругления первой ступени (`lays[0].kerb`): их
        // выход у ступеней бит-в-бит одинаков (отпечаток записи 28), у
        // одной ступени это её собственные скругления
        // ── Кромки слияний ──
        let merge_edges = merges::lay_merge_edges(
            map,
            prepared,
            &drawn,
            &nodal,
            shape.taper(),
            &mut streets,
            &mut sidewalks,
        );
        // ── Карманы у бордюра ──
        let kerb_pockets = pockets::lay_kerb_pockets(
            map,
            nodes,
            &nodal,
            junctions.row(),
            prepared,
            &mut streets,
            &mut sidewalks,
        );
        // ── Разворотные площадки ──
        let turning_circles =
            junctions::lay_turning_circles(map, nodes, prepared, &mut streets, &mut sidewalks);
        // ── Оси лент, траектории и порядок заливки ──
        // оси ленты, со стежками — до дороги, до которой OSM торец не довёл:
        // асфальт, краска, траектории, острова
        let ribbon = prepared.axes(Axis::Ribbon);
        // Широкие улицы поверх узких — см. доку модуля; ведущая узла — поверх
        // всех: её колея идёт через узел, и примыкание шире неё не должно её
        // закрыть.
        let leading = junctions.leading();
        let widths: Vec<f32> = drawn.iter().map(|road| road.width).collect();
        let order = fill_order(&widths, &leading, junctions.node_paint().junctions());
        // ── Направляющие островки, кольца и устья ──
        // направляющие островки у колец (`roads/gores.rs`) — до лент: к ним
        // дотягиваются двойные сплошные разделительных
        let gore_roads: Vec<gores::GoreRoad> = order
            .iter()
            .filter(|&&index| {
                let road = drawn[index];
                road.class == RoadClass::Street && !road.carves_navmesh()
            })
            .map(|&index| {
                gores::GoreRoad::new(
                    drawn[index],
                    &ribbon[index],
                    prepared.rings().leg_flow(index),
                )
            })
            .collect();
        // остров кольца из дуг — его замкнутая ось (`rings::Ring::path`)
        let ring_lawn_islands: Vec<&[Vec2]> = prepared
            .rings()
            .list
            .iter()
            .map(|ring| ring.path.as_slice())
            .collect();
        let ring_lawns = ring_island_lawns(&ring_lawn_islands);
        let ring_grass = ring_island_grass(&ring_lawn_islands, &map.grass);
        let mut gores = gores::Gores::of(&gore_roads, &ring_lawn_islands);
        // островки подходов — из скелета: они же в разрывах краски узлов
        gores.add_splitters(splitters);
        // клинья перед носами острых развилок улиц (`corners::fork_gore`)
        gores.add_forks(lays[0].kerb.fork_gores());
        // пустоты, зажатые проезжими частями, где ничего не замаплено (R20)
        let found_mouths = mouths(
            map,
            prepared,
            &ribbon,
            islands,
            &ring_lawn_islands,
            &gores,
            &lays[0].kerb,
        );
        // остриё устья, на которое легла зебра, отступает от неё: штриховка по
        // переходу ложилась поверх его полос (Тула, R20)
        let crossings: Vec<(Vec2, Vec2)> = junctions
            .node_paint()
            .marks()
            .zebras()
            .iter()
            .map(|zebra| (zebra.from, zebra.to))
            .collect();
        // зебра срезает только штриховку острия: асфальт устья под ней остаётся
        let hatchable_mouths = gores::mouths::clear_crossings(
            found_mouths.clone(),
            &crossings,
            node_paint::ZEBRA_LENGTH / 2.0 + MOUTH_CROSSING_GAP,
        );
        gores.add_mouths(&found_mouths, &hatchable_mouths);
        // ── Разрывы узлов и рампы слияний ──
        // три множества разрывов — каждому потребителю своё (`roads/junctions.rs`)
        let node_paint = junctions.node_paint();
        let marks = node_paint.marks();
        let asphalt = junctions.asphalt();
        let paint_breaks = junctions.paint();
        // каркасы половин у слияний сводятся в каркас продолжения
        // (`roads/merges.rs`) — по той же нарисованной оси, что и линии
        let ramps = merges::ramps(
            &prepared.merges().list,
            &drawn,
            &ribbon,
            &map.network,
            shape.taper(),
        );
        // ── Разделительные парных половин ──
        // где у пары половин кончается разделительная — осевая слияния доходит
        // до неё: торцы асфальтовой середины и носы газона, по улицам половин
        let street_of = |road: usize| map.network.street_of(road).map(|(street, _)| street);
        // разделительные парных половин (`roads/medians.rs`): асфальт — до лент
        // половин, под ними; газон с бордюром — в свой слой над тротуарами;
        // открываются по базе — у перекрёстка, кто бы его ни вёл. Двойную
        // сплошную красит не он, а мы — по его списку
        // что замыкает зазор впереди торца разделительной: проезжие части из
        // узлов её половин рядом с торцом — связка, разворот
        let closer = |halves: [usize; 2], tip: Vec2, heading: Vec2, reach: f32| {
            let mut near: Vec<usize> = halves
                .iter()
                .flat_map(|&half| nodal[half].iter())
                .filter(|point| point.distance(tip) <= 2.0 * reach && nodes.is_shared(**point))
                .flat_map(|point| nodes.roads_at(*point).iter().copied())
                .filter(|road| !halves.contains(road) && is_carriageway(drawn[*road]))
                .collect();
            near.sort_unstable();
            near.dedup();
            medians::closing_reach(
                tip,
                heading,
                reach,
                near.iter().map(|&road| ribbon[road].as_ref()),
            )
        };
        let median_drawing = medians::draw(
            prepared.pairs(),
            &medians::MedianInputs {
                base: junctions.median_base(),
                paint: paint_breaks,
                zebras: marks.zebras(),
                half: &|road| prepared.road(road).width / 2.0,
                markings: style.markings,
                pure_merge: &|at| prepared.merges().is_pure_node(at),
                reach_gores: &|midline| gores.reach(midline),
                street_of: &street_of,
                closer: &closer,
                on_bridge: &|road| drawn[road].bridge,
            },
            &mut streets,
            &mut sidewalks,
            &mut median_grass,
            bridges.fills(),
        );
        for (midline, painted, on_deck) in &median_drawing.painted {
            painter.paint_median(midline, painted, *on_deck);
        }
        // за узлом слияния, до разделительной его пары: асфальт до носа газона —
        // под лентами половин — и осевая продолжения
        streets.set_lanes(None);
        for merge in prepared.merges().list.iter().filter(|merge| merge.pure) {
            let halves = merge.halves.map(street_of);
            let ends: Vec<merges::MedianEnd> = median_drawing
                .ends
                .iter()
                .filter(|(pair, _)| {
                    pair.iter()
                        .all(|street| street.is_some() && halves.contains(street))
                })
                .map(|&(_, end)| end)
                .collect();
            for shape in merges::nose_fill(
                merge,
                &ribbon,
                &map.network,
                &ends,
                &median_drawing.lawn_kerbs,
            ) {
                push_shape(&mut streets, shape, ROAD_COLOR.to_linear());
            }
            if style.markings {
                // из торца нарисованной осевой продолжения и по её касательной
                let continuation = drawn[merge.street];
                let offset =
                    paint::axis_offset(continuation, lane_count(continuation), map.traffic_side);
                let lead = merges::LeadIn::of(merge, &ribbon, offset.unwrap_or(0.0));
                let axis = merges::merge_axis(merge, &drawn, &ribbon, &map.network, &ends, lead);
                painter.paint_merge_axis(&axis, lane_count(drawn[merge.street]));
            }
        }
        // асфальт от торца полотна до носа газона рядом — или вперёд за торец
        streets.set_lanes(None);
        for cap in median_drawing.bed_caps() {
            push_shape(&mut streets, cap, ROAD_COLOR.to_linear());
        }
        let network_time = started.elapsed();

        // ── Площади и островки до лент ──
        let (road_islands, road_bites) = islands::lay_areas(
            map,
            prepared,
            &drawn,
            &ribbon,
            islands,
            &mut streets,
            &mut sidewalks,
        );
        // ── Тротуары колец ──
        let island_insets = rings::lay_ring_island_bands(prepared, roads, style, &mut sidewalks);
        // подходы колец: их полосы тротуара копятся ниже для газона острова
        let at_ring = rings::approach_flags(prepared, drawn.len());
        let mut approach_bands: Vec<ApproachBand> = Vec::new();
        // ── Заливка колец и швы настилов ──
        // асфальт кольца — одной замкнутой заливкой по его оси, на месте первой
        // не-мостовой дуги в порядке заливки (R15): дуги, кончающиеся в узлах с
        // подходами, клались лентами с прямыми торцами, и на кривизне торцы
        // соседних дуг расходились веером — по внешней кромке клинья-щели;
        // первую дугу для вкладов ленты считает `ribbon_lane::fill_arcs`
        let seam_gaps = bridges::seam_dash_gaps(prepared, &drawn, &ribbon);
        // ── Ленты ── главный проход по улицам в порядке заливки: асфальт,
        // тротуары, обочины, краска, клинья и мосты — улица за улицей
        //
        // Обочины — куски улиц в потоках, обе ступени одним пулом: улица от
        // улицы не зависит (вход — только чтение: подготовка, ленты, торцы,
        // укусы, газоны), улицы куска пишут прямо в частичный вклад куска.
        // По-штучные вклады улиц стоили проходу аллокационный хвост и
        // раскладку ста тысяч результатов (разрез записи 46: длительности
        // улиц короткие и ровные — хвост держала сантехника результатов);
        // куски — те же, что у раскладки ленты ниже: аппенды частичных
        // вкладов в порядке кусков дают порядок заливки — треугольников,
        // лоскутов и находки, как у серийной укладки.
        let chunks = ribbon_lane::chunks(&drawn, &order);
        let items: Vec<(usize, usize)> = (0..lays.len())
            .flat_map(|step| (0..chunks.len()).map(move |chunk| (step, chunk)))
            .collect();
        let partials: Vec<verges::StreetVerges> = in_parallel(&items, |(step, chunk)| {
            let lay = &lays[*step];
            let kerb = &lay.kerb;
            let verge = lay.verge;
            let mut partial = verges::StreetVerges::new();
            for position in chunks[*chunk].clone() {
                let index = order[position];
                verges::street_verge_geometry(
                    &mut partial,
                    prepared,
                    &drawn,
                    &ribbon,
                    &meadows,
                    &road_bites,
                    kerb.butt(index),
                    index,
                    verge,
                );
            }
            partial
        });
        let mut partials = partials.into_iter();
        for lay in lays.iter_mut() {
            lay.verge_chunks = partials.by_ref().take(chunks.len()).collect();
        }
        // Вклад кусков улиц — в потоках (`roads/ribbon_lane.rs`): улица от
        // улицы не зависит (вход — только чтение: подготовка, ленты, торцы,
        // укусы, газоны), записи идут в свои сборщики вклада. Раскладка ниже
        // дописывает вклады в порядке кусков — порядок кусков каждого слоя,
        // краски, колеи и стоянок тот же, что у серийного прохода. Куски
        // разрезаны на мостах: их сборщики (`Bridges`) общие, и ветка моста
        // остаётся серийной на своём месте в порядке заливки.
        let fill_arc = ribbon_lane::fill_arcs(prepared, &drawn, &order, &road_islands.outlines);
        let lane = ribbon_lane::RibbonLane {
            prepared,
            drawn: &drawn,
            order: &order,
            ribbon: &ribbon,
            ramps: &ramps,
            stations,
            junctions,
            seam_gaps: &seam_gaps,
            kerb: &lays[0].kerb,
            style,
            traffic_side: map.traffic_side,
            at_ring: &at_ring,
            outlines: &road_islands.outlines,
            grounds: &grounds,
        };
        let parts: Vec<ribbon_lane::ChunkRibbon> =
            in_parallel(&chunks, |range| lane.chunk(range.clone(), &fill_arc));
        for (chunk, (part, range)) in parts.into_iter().zip(chunks).enumerate() {
            let position = range.start;
            let index = order[position];
            let road = drawn[index];
            if road.bridge {
                // мост — кусок из одной улицы: краска и колея из вклада, тело —
                // серийно. Бордюр и тень — мосту; заливка — здесь, в порядке
                // улиц. Клин на шве земля/мост сужает настил вместе с бортиком
                // (R30); торец, севший на подход, — прямой, как бортик (R14,
                // `trimmed`). Бортика со стороны пары нет: между
                // настилами-половинами лежит разделительная в слое настилов
                // (`medians::draw`), и внутренние бортики светились бы по её
                // краям разделителем, которого на подходе нет (R30) — куски,
                // как у полосы тротуара, только по мощёной разделительной: над
                // газоном бортик и есть край моста
                if road_islands.outlines[index] {
                    continue;
                }
                let points: &[Vec2] = &ribbon[index];
                // колея гаснет по разрывам асфальта; у ведущей узла их там нет
                let breaks = asphalt.of(index);
                let lanes = road_lanes(road);
                let color = fill_color(road);
                ruts.lanes.extend(part.ruts);
                if let Some(painter_part) = part.painter {
                    painter.append_from(&painter_part);
                }
                let ends = prepared.taper_ends(index);
                let [head, body, tail] = if ends == [None; 2] {
                    [None, None, None]
                } else {
                    tapers::split(points, ends.map(|end| end.map(|taper| taper.length)))
                };
                let body: &[Vec2] = body.as_deref().unwrap_or(points);
                let butt = lays[0].kerb.butt(index);
                let stitched_end = prepared.stitched_end(index);
                let trimmed = [
                    head.is_some() || (butt[0] && !stitched_end[0]),
                    tail.is_some() || (butt[1] && !stitched_end[1]),
                ];
                let seam = bridges::seam_ends(&drawn, prepared.nodes(), index);
                let banded = [trimmed[0] || seam[0], trimmed[1] || seam[1]];
                let trimmed = banded;
                let wedges = tapers::wedge_run(&head, &tail, ends);
                let curb = road.curb_reach() - road.width / 2.0;
                let head_length = head.as_deref().map_or(0.0, polyline_length);
                let pieces = prepared.pairs().curb_pieces(
                    index,
                    prepared.stitch_offset(index) - head_length,
                    polyline_length(body),
                );
                let total_length = polyline_length(points);
                let curb_wedges: Vec<(&[Vec2], [[f32; 2]; 2])> = wedges
                    .iter()
                    .map(|&(path, taper, end)| {
                        let seam_half = drawn[taper.seam].width / 2.0;
                        let middle = wedge_middle(total_length, polyline_length(path), end);
                        let paired = prepared
                            .pairs()
                            .beside_paved(index, middle, 0.0)
                            .map(|left| usize::from(!left));
                        let halves = wedge_halves(taper.sides, end, |side| {
                            let curb = if paired == Some(side) { 0.0 } else { curb };
                            [seam_half + curb, road.width / 2.0 + curb]
                        });
                        (path, halves)
                    })
                    .collect();
                bridges.push_deck(
                    index,
                    points,
                    body,
                    head_length,
                    pieces.as_deref(),
                    &curb_wedges,
                    road,
                );
                let fills = bridges.fills();
                fills.set_lanes(lanes);
                push_street_fill(fills, body, road.width, color.to_linear(), breaks, trimmed);
                fills.set_lanes(lanes);
                tapers::push_asphalt(
                    fills,
                    &wedges,
                    &drawn,
                    road,
                    body,
                    breaks,
                    lanes,
                    color.to_linear(),
                    map.traffic_side,
                );
                // фартук асфальта под торцом настила-улицы на шве, на подход —
                // в слое улиц, под их краской (`bridges.rs`)
                bridges.push_seam_aprons(
                    &mut streets,
                    &wedges,
                    seam,
                    points,
                    road,
                    pieces.as_deref(),
                );
                continue;
            }
            streets.append(&part.streets);
            sidewalks.append(&part.sidewalks);
            alleys.append(&part.alleys);
            unpaved.append(&part.unpaved);
            if let Some(painter_part) = part.painter {
                painter.append_from(&painter_part);
            }
            ruts.lanes.extend(part.ruts);
            approach_bands.extend(part.approach);
            // улицы больших стоянок — вкладом; улицы внутри площадки идут в
            // порядке заливки
            grounds.accept(part.grounds);
            // обочина до отдельного тротуара — частичный вклад куска
            // (`verge_chunks`): раскладка здесь, в порядке кусков — на
            // каждую ступень в её уклад; улицы внутри куска уже в порядке
            // заливки, так что порядок совпадает с по-уличной раскладкой
            for lay in lays.iter_mut() {
                lay.verge_chunks[chunk].apply(
                    [&mut lay.verges, &mut lay.verge_lawns, &mut lay.verge_yards],
                    &mut lay.scraps,
                    &mut lay.pieces,
                );
            }
        }
        // ── Фартуки мостиков ──
        aprons::lay_slit_aprons(
            &drawn,
            &ribbon,
            prepared,
            &mut bridges,
            &mut sidewalks,
            &mut alleys,
        );
        // ── Краска узлов и колея траекторий ──
        // (лоскуты — в финале ступени: их находка по вкладам ступени)
        // Краска читает уложенный асфальт — и ей нужен слой со скруглениями: в
        // общем сборщике их нет, они в укладах ступеней (бит-в-бит одинаковые,
        // отпечаток записи 28). Временный слой первой ступени — ровно то, что
        // читала одноступенчатая сборка; сама краска пишет в painter и колею
        let mut node_streets = lays[0].streets.clone();
        node_streets.append(&streets);
        node_paint::paint_junction_marks(
            marks,
            &node_streets,
            &mut painter,
            turns,
            &mut ruts,
            paint_breaks,
            style,
        );
        // ── Поверх асфальта: островки, трамвай, стоянки ──
        // направляющие островки у колец: асфальт — в слой улиц, поверх тротуаров,
        // разметка — в слой краски, своим мешем выше асфальта стоянок
        // (`roads/gores.rs`)
        gores.push_asphalt(&mut streets, ROAD_COLOR.to_linear());
        let (tram_tracks, tram_bands) = tram_band::lay_tram_bands(
            map,
            &drawn,
            &nodal,
            &ribbon,
            &median_drawing,
            &mut bridges,
            &mut streets,
        );
        let mut lot_layers = grounds.layers(&style, &gores, &median_drawing.paved);
        for shape in &road_islands.kerbs {
            push_shape(
                &mut lot_layers.sidewalks,
                shape.clone(),
                SIDEWALK_COLOR.to_linear(),
            );
        }
        if style.markings {
            for (island, across) in gores.islands() {
                painter.paint_island(island, across);
            }
        }

        // ── Путепроводы и переезды ──
        // путепроводы: плита, парапет и тень — в тех же мостовых слоях; путь
        // поверх плиты кладёт `map::rail`. Трамвай на мосту едет по мосту улицы
        let mut track_bridges = 0;
        for rail in map.rails.iter().filter(|rail| rail.is_track_bridge()) {
            bridges.push_track(rail);
            track_bridges += 1;
        }
        // переезды в одном уровне — по оси ленты и ширине улицы как рисуются,
        // с тротуаром там, где он рисуется (`rail::mesh_level_crossings`).
        // Вопрос — нарисованному: переезд через тротуар стоит в слоте дорожки
        // (`network::driveway_crossings`) и настил кладёт
        let crossed: Vec<CrossedStreet> = (0..drawn.len())
            .filter(|&index| drawn[index].is_at_grade())
            .map(|index| CrossedStreet {
                axis: &ribbon[index],
                reach: [0, 1].map(|side| {
                    drawn[index].width / 2.0 + prepared.sidewalk_on(index, side).unwrap_or(0.0)
                }),
            })
            .collect();
        let (rail_crossings, level_crossings) = mesh_level_crossings(&map.rails, &crossed);
        // ── Газон островов колец ──
        rings::cover_ring_islands(prepared, &island_insets, &approach_bands, &mut median_grass);
        // ── Лента крепостной стены ──
        walls::lay_wall_ribbons(map, &mut wall_ribbons);

        RoadCommon {
            lays,
            streets,
            sidewalks,
            alleys,
            unpaved,
            median_grass,
            ring_lawns,
            ring_grass,
            wall_ribbons,
            rail_crossings,
            bridges,
            painter,
            lot_layers,
            drawn,
            ribbon,
            gores,
            road_islands,
            ruts,
            tram_tracks,
            merge_edges,
            kerb_pockets,
            turning_circles,
            tram_bands,
            track_bridges,
            level_crossings,
            network_time,
            started,
        }
    }

    /// Общие слои сборки **сплит-двери**: чисто общие переносятся из своих
    /// сборщиков как есть, смешанные (скругления + общая середина) —
    /// скругления первой ступени с дописанной один раз серединой, краска и
    /// мосты — один раз. Скругления ступеней бит-в-бит (отпечаток записи 28),
    /// поэтому мир рисует уклад ступени 0, а ступень 1 свои смешанные
    /// сборщики выбрасывает. Вынутые сборщики обнуляются: финалы ступеней
    /// ([`RoadCommon::finish_step_split`]) общих сборщиков не читают.
    /// Возвращает слои, их сумму и краску в ней — отчёты ступеней прибавляют
    /// счётчики к своим слоям, чтобы строка `road meshing:` осталась суммой
    /// полной картины ступени (как у слитой двери — [`build_roads_from`]).
    pub(super) fn take_common_layers(&mut self) -> CommonLayers {
        // середина умирает в копии: содержимое переехало в смешанный слой,
        // и держать её до конца финалов незачем
        let streets_mid = std::mem::take(&mut self.streets);
        let sidewalks_mid = std::mem::take(&mut self.sidewalks);
        let alleys_mid = std::mem::take(&mut self.alleys);
        let unpaved_mid = std::mem::take(&mut self.unpaved);
        let lay = &mut self.lays[0];
        let mut streets = std::mem::take(&mut lay.streets);
        streets.append(&streets_mid);
        let mut sidewalks = std::mem::take(&mut lay.sidewalks);
        sidewalks.append(&sidewalks_mid);
        let mut alleys = std::mem::take(&mut lay.alleys);
        alleys.append(&alleys_mid);
        let mut unpaved = std::mem::take(&mut lay.unpaved);
        unpaved.append(&unpaved_mid);
        let median_grass = std::mem::take(&mut self.median_grass);
        let ring_lawns = std::mem::take(&mut self.ring_lawns);
        let ring_grass = std::mem::take(&mut self.ring_grass);
        let wall_ribbons = std::mem::take(&mut self.wall_ribbons);
        let rail_crossings = std::mem::take(&mut self.rail_crossings);
        let lot_layers = std::mem::take(&mut self.lot_layers);
        // мосты клонируются, а не вынимаются: отчёты ступеней читают их счёт
        let paint = Vec::from(self.painter.clone().layers());
        let paint_vertices = paint
            .iter()
            .map(|layer| layer.mesh.builder.vertex_count())
            .sum();
        let mut layers: Vec<LayerMesh> = [
            (
                ring_lawns,
                Z_RING_ISLAND,
                "ring_islands",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                ring_grass,
                Z_RING_GRASS,
                "ring_island_grass",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
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
                median_grass,
                Z_ROAD_MEDIAN,
                "road_medians",
                MaterialSpec::Surface(SurfaceKind::Grass),
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
            (
                lot_layers.sidewalks,
                Z_LOT_SIDEWALK,
                "lot_sidewalks",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
            (
                lot_layers.lines,
                Z_LOT_LINES,
                "lot_lines",
                MaterialSpec::Flat,
            ),
            (wall_ribbons, Z_WALL, "walls", MaterialSpec::Flat),
            (
                rail_crossings,
                Z_RAIL_CROSSING,
                "rail_crossings",
                MaterialSpec::Flat,
            ),
        ]
        .into_iter()
        .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
        .chain(self.bridges.clone().layers())
        .collect();
        layers.sort_by(|a, b| a.z.total_cmp(&b.z));
        let vertices = layers
            .iter()
            .map(|l| l.builder.vertex_count())
            .sum::<usize>()
            + paint_vertices;
        CommonLayers {
            layers,
            vertices,
            paint_vertices,
            paint,
        }
    }

    /// Финал ступени **сплит-двери**: только её ступенезависимые слои
    /// — обочины из её уклада и мощёные клинья зелени отдельным слоем
    /// (`sidewalk_scraps`; слитая дверь возвращает хвост в тротуары —
    /// [`build_roads_from`], порядок кусков тот же). Отчёт —
    /// по полной картине ступени: свои слои плюс общие
    /// ([`RoadCommon::take_common_layers`]) — та же строка, что у слитой
    /// сборки.
    pub(super) fn finish_step_split(
        &self,
        skeleton: &RoadSkeleton<'_>,
        lay: StepLay,
        common_vertices: usize,
        paint_vertices: usize,
    ) -> StepRoadBuild {
        let RoadSkeleton {
            map,
            style,
            prepared,
            islands,
            junctions,
            turns,
            ..
        } = skeleton;
        let map = *map;
        let style = *style;
        // Уклад ступени пришёл по значению ([`build_roads_steps`]): общие
        // сборщики ступени пусты (их содержимое у [`RoadCommon::
        // take_common_layers`]), ступенезависимое — её собственное.
        let StepLay {
            verge: _,
            kerb,
            streets: _,
            alleys: _,
            unpaved: _,
            sidewalks: _,
            mut verges,
            verge_lawns,
            verge_yards,
            mut scraps,
            mut pieces,
            verge_chunks: _,
        } = lay;
        // ── Лоскуты ── находка по вкладам ступени (скругления и улицы уже в
        // списке): мощение после лент, как у слитой сборки. Мощёные клинья
        // зелени едут в свой слой тротуарного материала: у слитой сборки они
        // лежали в тротуарах после общей середины, и слой тротуаров мира —
        // общий, ступени их не прятала бы
        scraps::pave_scraps(
            &kerb,
            &self.gores,
            &self.drawn,
            prepared,
            &self.ribbon,
            &self.road_islands,
            &mut scraps,
        );
        let mut sidewalk_scraps = MeshBuilder::with_surface_coords();
        let lawn_scraps = scraps::lay_green_scraps(
            map,
            &mut scraps,
            &mut verges,
            &mut sidewalk_scraps,
            &mut pieces,
        );
        // ── Сборка слоёв и отчёт ──
        let layers: Vec<LayerMesh> = [
            (
                verge_lawns,
                Z_ROAD_VERGE_LAWN,
                "road_verge_lawns",
                MaterialSpec::Surface(SurfaceKind::Grass),
            ),
            (
                verge_yards,
                Z_ROAD_VERGE_YARD,
                "road_verge_yards",
                MaterialSpec::Surface(SurfaceKind::Yard),
            ),
            (
                verges,
                Z_ROAD_VERGE,
                "road_verges",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
            (
                sidewalk_scraps,
                Z_SIDEWALK,
                "sidewalk_scraps",
                MaterialSpec::Surface(SurfaceKind::Sidewalk),
            ),
        ]
        .into_iter()
        .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
        .collect();
        // что скругления насчитали — поля отчёта о них (`corners.rs`)
        let kerbs = kerb.counts();
        let report = RoadReport {
            style,
            junctions: junctions.counts(),
            paint_lines: self.painter.lines,
            paint_vertices,
            kerb_pockets: self.kerb_pockets,
            turning_circles: self.turning_circles,
            turns: turns.maneuvers,
            arrows: if style.arrows { turns.arrows.len() } else { 0 },
            kerb_returns: kerbs.kerb_returns,
            sidewalk_returns: kerbs.sidewalk_returns,
            tile_overlaps: kerbs.tile_overlaps,
            bends: kerbs.bends,
            outer_corners: kerbs.outer_corners,
            noses: kerbs.noses,
            lawn_scraps,
            drawn: prepared.stats(),
            bridges: self.bridges.count(),
            track_bridges: self.track_bridges,
            level_crossings: self.level_crossings,
            gores: self.gores.count(),
            road_islands: [
                self.road_islands.refuges,
                self.road_islands.kerbs.len() - self.road_islands.refuges,
                self.road_islands.carriageways.len(),
                self.road_islands.walkways.len(),
            ],
            merge_edges: self.merge_edges,
            islands: islands.len(),
            tram_bands: self.tram_bands,
            vertices: layers
                .iter()
                .map(|l| l.builder.vertex_count())
                .sum::<usize>()
                + common_vertices,
            network: self.network_time,
            elapsed: self.started.elapsed(),
        };
        StepRoadBuild {
            layers,
            report,
            ruts: self.ruts.clone(),
            tram: self.tram_tracks.clone(),
            pieces,
        }
    }
}

/// Общие слои сплит-сборки и счётчики для отчётов ступеней — выход
/// [`RoadCommon::take_common_layers`].
pub(super) struct CommonLayers {
    pub(super) layers: Vec<LayerMesh>,
    pub(super) paint: Vec<paint::PaintLayer>,
    pub(super) vertices: usize,
    pub(super) paint_vertices: usize,
}
