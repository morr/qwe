//! Дороги на большой стоянке — бордюром и разметкой **поверх** её асфальта.
//!
//! Стоянка лежит выше дорожных лент (`Z_PARKING` над `Z_ROAD`) и кроет всё,
//! что на неё заходит. Для двора это верно: асфальт площадки и есть проезд. У
//! большой стоянки ([`parking::is_ground`]) сквозь площадку идёт настоящая
//! дорога ([`parking::is_through`]) — у ТРЦ «Макси» бульвар с односторонним
//! движением и тремя кольцами, — и спрятанная, она оставляла восемь гектаров
//! штриховки без единого ориентира (отчёт автора).
//!
//! Асфальт у такой дороги тот же, что у площадки, и второй раз не кладётся:
//! дорогу на площадке показывает её **бордюр** ([`parking::kerb_width`], слой
//! `lot_sidewalks`). Он считается **полигоном, а не лентами**: полосы сквозных
//! дорог с бордюром, минус асфальт всех улиц площадки (проезд ряда прорезает в
//! бордюре устье, и вдоль бульвара тот выходит островками у торцов рядов) и
//! минус направляющие островки у колец, в пределах контура площадки,
//! **выпущенного на [`KERB_OVERHANG`]** — иначе бордюр встаёт к тротуару улицы,
//! с которой дорога на площадку заходит, ступенькой. Лентами он был сперва —
//! асфальт каждой дороги лежал вторым слоем поверх бордюров всех остальных, — и
//! у колец, где съезды расходятся веером, ленты рубили друг друга в обрывки с
//! рваными торцами, а островок кольца оставался кольцом (отчёт автора). У
//! булевой разности рваных краёв нет, остров кольца залит, а обрезки у́же двух
//! [`KERB_OPENING`] снимает размыкание.
//!
//! Между двумя встречными полотнами, идущими бок о бок, бордюра нет: там
//! **двойная сплошная** (слой `lot_lines`), как рисуют Яндекс и 2ГИС. Пару
//! полотен и её середину находит сеть (`roads/network/pairs.rs`) — для всего
//! города, а не только для стоянки; здесь берётся её кусок над площадкой
//! ([`LotLayers`]) и дотягивается до островка у кольца (`Gores::reach`). Своя
//! линия нужна потому, что слой краски улиц лежит под асфальтом стоянки.
//!
//! **Клин между расходящимися у кольца полотнами здесь не считается** — это
//! свойство сети у кольца, а не стоянки, и живёт оно в `roads/gores.rs`: там он
//! асфальт со штриховкой, для любого кольца города. Стоянка только вычитает
//! островки из своего бордюра и дотягивает до них осевую.
//!
//! Замкнут ли way, спрашивается **у сырых точек OSM** ([`Grounds::push`]), а не
//! у нарисованной оси: сглаживание срезает угол на шве замкнутого way, и концы
//! нарисованного кольца расходятся на метры.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle};

use super::gores::Gores;
use super::network::pairs::{Median, PAIR_MIN, samples};
use super::{LOT_LINE_COLOR, RoadStyle, SIDEWALK_COLOR};
use crate::map::meshing::{MeshBuilder, RibbonJoin};
use crate::map::osm::model::{
    MapData, PolyArea, RoadLine, point_in_area, polyline_length, ring_bounds,
};
use crate::map::parking::{is_ground, is_through, kerb_width};
use crate::map::shapes::{
    ARC, Contour, RING_EPSILON, Shape, area_contours, contour_area, is_ring, oriented, push_shape,
    stroke,
};

/// Размыкание бордюра, м: обрезок у́же двух таких снимается, углы скругляются.
/// Бордюр сам 1.2 м и больше — ему это ничего не стоит.
const KERB_OPENING: f32 = 0.3;
/// Обрезок бордюра мельче этого, м², не рисуется: островок у торца ряда — от
/// четырнадцати, а мельче выходят огрызки там, где дорогу режет кромка площадки.
const MIN_KERB_AREA: f32 = 6.0;
/// На сколько бордюр выпущен за контур площадки, м, — до тротуара улицы, с
/// которой дорога на неё заходит.
const KERB_OVERHANG: f32 = 2.0;
/// Кусок разделительной над площадкой короче этого, м, — не рисуется.
const MEDIAN_MIN: f32 = PAIR_MIN;
/// Двойная сплошная: ширина линии и расстояние между осями линий, м. Шире
/// настоящей (0.15 и 0.3) — иначе на отдалении обе сливаются в волосок.
const DOUBLE_LINE_WIDTH: f32 = 0.2;
const DOUBLE_LINE_GAUGE: f32 = 0.5;

/// Улица у большой стоянки — так, как она нарисована (сглаженная ось).
struct Street {
    path: Vec<Vec2>,
    width: f32,
    /// Бордюр с одной стороны — у сквозной дороги, чья ось зашла на площадку.
    kerb: Option<f32>,
}

struct Ground<'a> {
    lot: &'a PolyArea,
    low: Vec2,
    high: Vec2,
    streets: Vec<Street>,
}

/// Большие стоянки карты и улицы, что на них заходят.
pub(super) struct Grounds<'a>(Vec<Ground<'a>>);

/// Два слоя поверх асфальта стоянки: бордюры сквозных дорог и двойная сплошная
/// между встречными полотнами.
pub(super) struct LotLayers {
    pub sidewalks: MeshBuilder,
    pub lines: MeshBuilder,
}

impl<'a> Grounds<'a> {
    pub fn of(map: &'a MapData) -> Self {
        Self(
            map.parking
                .iter()
                .filter(|lot| is_ground(lot))
                .map(|lot| {
                    let (low, high) = ring_bounds(&lot.outer);
                    Ground {
                        lot,
                        low,
                        high,
                        streets: Vec::new(),
                    }
                })
                .collect(),
        )
    }

    /// Улица `road`, нарисованная по `path`, — каждой большой стоянке, до
    /// которой она достаёт.
    pub fn push(&mut self, road: &RoadLine, path: &[Vec2]) {
        if self.0.is_empty() || path.len() < 2 {
            return;
        }
        let kerb = is_through(road).then(|| kerb_width(road));
        let reach = road.width / 2.0 + kerb.unwrap_or_default();
        // замкнутость — по сырым точкам OSM, а не по нарисованной оси: свойство
        // way, а не стиля рисования. Сглаживание кольцо замыкает
        // (`smooth_pinned` идёт по циклу), так что дозамыкание ниже —
        // страховка на случай оси, пришедшей другим путём
        let mut closed = path.to_vec();
        if let (true, Some(first)) = (is_ring(&road.points), closed.first().copied())
            && closed
                .last()
                .is_some_and(|last| last.distance(first) > RING_EPSILON)
        {
            closed.push(first);
        }
        let path = closed.as_slice();
        let (low, high) = ring_bounds(path);
        for ground in &mut self.0 {
            if low.cmpgt(ground.high + reach).any() || high.cmplt(ground.low - reach).any() {
                continue;
            }
            // бордюр — только дороге, чья ось идёт по площадке: у улицы вдоль
            // кромки он свой, тротуаром, и на площадку не заходит
            let inside = kerb.is_some() && enters(path, ground.lot);
            ground.streets.push(Street {
                path: path.to_vec(),
                width: road.width,
                kerb: kerb.filter(|_| inside),
            });
        }
    }

    /// Бордюры и двойные сплошные площадок. `medians` — асфальтовые
    /// разделительные города (`roads/network/pairs.rs`): середина и
    /// расстояние между осями половин.
    pub fn layers(&self, style: &RoadStyle, gores: &Gores, medians: &[Median]) -> LotLayers {
        let mut layers = LotLayers {
            sidewalks: MeshBuilder::with_surface_coords(),
            lines: MeshBuilder::default(),
        };
        for ground in &self.0 {
            if !ground.streets.iter().any(|street| street.kerb.is_some()) {
                continue;
            }
            let mut medians = on_lot(medians, ground);
            for (midline, _) in &mut medians {
                gores.reach(midline);
            }
            medians.retain(|(midline, _)| midline.len() >= 2);
            if style.markings {
                for (midline, _) in &medians {
                    layers.lines.push_rails(
                        midline,
                        DOUBLE_LINE_GAUGE,
                        DOUBLE_LINE_WIDTH,
                        LOT_LINE_COLOR.to_linear(),
                        RibbonJoin::Round,
                    );
                }
            }
            if style.sidewalks {
                for shape in kerbs(ground, &medians, gores) {
                    push_shape(&mut layers.sidewalks, shape, SIDEWALK_COLOR.to_linear());
                }
            }
        }
        layers
    }
}

/// Бордюры площадки — полосы сквозных дорог и острова колец, минус асфальт
/// всех улиц, разделительные и **направляющие островки** у колец
/// (`roads/gores.rs`: там асфальт со штриховкой, бордюра между полотнами нет),
/// разомкнутые на [`KERB_OPENING`].
fn kerbs(ground: &Ground, medians: &[(Vec<Vec2>, f32)], gores: &Gores) -> Vec<Shape> {
    let mut bands: Vec<Contour> = Vec::new();
    let mut asphalt: Vec<Contour> = Vec::new();
    let mut islands: Vec<Contour> = Vec::new();
    for street in &ground.streets {
        let ring = is_ring(&street.path);
        let band = stroke(&street.path, street.width, LineCap::Round(ARC), ring);
        if let Some(kerb) = street.kerb {
            bands.extend(stroke(
                &street.path,
                street.width + 2.0 * kerb,
                LineCap::Round(ARC),
                ring,
            ));
            // остров кольца — весь, а не ободком вдоль полотна
            if ring {
                islands.push(oriented(&street.path[1..], true));
            }
        }
        asphalt.extend(band);
    }
    for (midline, width) in medians {
        asphalt.extend(stroke(midline, *width, LineCap::Butt, is_ring(midline)));
    }
    let lot: Vec<Contour> = area_contours(ground.lot);
    let round = LineJoin::Round(ARC);
    // бордюр выпущен за контур на [`KERB_OVERHANG`]: обрезанный ровно по
    // нему, он вставал к тротуару улицы, с которой дорога заходит на
    // площадку, ступенькой — контур там скруглён замыканием, а тротуар нет
    let reach = vec![lot].outline(&OutlineStyle::new(KERB_OVERHANG).line_join(round.clone()));
    let mut cut = asphalt;
    cut.extend(gores.contours().cloned());
    bands.extend(islands);
    bands
        .overlay(&cut, OverlayRule::Difference, FillRule::NonZero)
        .overlay(&reach, OverlayRule::Intersect, FillRule::NonZero)
        .outline(&OutlineStyle::new(-KERB_OPENING).line_join(round.clone()))
        .outline(&OutlineStyle::new(KERB_OPENING).line_join(round))
        .into_iter()
        .filter(|shape| {
            shape
                .first()
                .is_some_and(|outer| contour_area(outer) >= MIN_KERB_AREA)
        })
        .collect()
}

/// Куски середин разделительных над площадкой — не короче [`MEDIAN_MIN`], с
/// расстоянием между осями половин.
fn on_lot(medians: &[Median], ground: &Ground) -> Vec<(Vec<Vec2>, f32)> {
    let mut found = Vec::new();
    for median in medians {
        let (low, high) = ring_bounds(&median.midline);
        if low.cmpgt(ground.high).any() || high.cmplt(ground.low).any() {
            continue;
        }
        let apart = median.apart();
        // по пробам, а не по вершинам: прямая середина — две точки, и обе
        // бывают за площадкой
        let probes: Vec<Vec2> = samples(&median.midline)
            .into_iter()
            .map(|(_, at, _)| at)
            .collect();
        for run in
            probes.chunk_by(|a, b| point_in_area(*a, ground.lot) == point_in_area(*b, ground.lot))
        {
            if point_in_area(run[0], ground.lot) && polyline_length(run) >= MEDIAN_MIN {
                found.push((run.to_vec(), apart));
            }
        }
    }
    found
}

/// Заходит ли ось на площадку — пробами через
/// [`PROBE_STEP`](super::network::pairs::PROBE_STEP).
fn enters(path: &[Vec2], lot: &PolyArea) -> bool {
    samples(path)
        .iter()
        .any(|(_, at, _)| point_in_area(*at, lot))
}
