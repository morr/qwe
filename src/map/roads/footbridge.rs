//! Мостик (`bridge=yes` на дорожке класса Alley) и его «вдоль» настила:
//! предикат, ручка [`FOOTBRIDGE_ALONG`] и тест [`runs_along`].
//!
//! «Идёт вдоль» — одно понятие с двумя читателями: `axis.rs` выносит по
//! нему мостик, замапленный внутри настила, к кромке настила
//! (`place_bridge_sidewalks`), `aprons.rs` мостит щель между настилом и
//! мостиком рядом (`slit_aprons`).

use bevy::prelude::*;

use super::drawn::Drawn;
use super::network::RoadNodes;
use super::ribbons::BAND_SEAM_OVERLAP;
use super::tapers;
use crate::map::meshing::{MeshBuilder, miter_offsets};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine};

/// Мостик: дорожка класса Alley по мосту. Арка (`passage`) идёт сквозь дом,
/// а не над настилом, — мостиком не считается.
pub(super) fn is_footbridge(road: &RoadLine) -> bool {
    road.bridge && road.class == RoadClass::Alley && !road.passage
}

/// Настил улицы — то, вдоль чего мостик идёт и из чего выносится к кромке.
/// Аркой (`passage`) не считается, по той же причине.
pub(super) fn is_street_deck(road: &RoadLine) -> bool {
    road.bridge && road.class == RoadClass::Street && !road.passage
}

/// Дорожка на мостик расширяется в него клином (S5)
pub(super) fn lay_flares(
    fill: &mut MeshBuilder,
    drawn: &[&RoadLine],
    prepared: &Drawn,
    road: &RoadLine,
    points: &[Vec2],
    index: usize,
    color: Color,
) {
    if road.class != RoadClass::Alley || road.passage {
        return;
    }
    let reach = footbridge_reach(drawn, prepared.nodes(), index);
    for (end, reach) in reach.into_iter().enumerate() {
        if let Some(reach) = reach.filter(|&reach| reach > road.width / 2.0) {
            push_path_flare(
                fill,
                points,
                end,
                [road.width / 2.0, reach],
                color.to_linear(),
            );
        }
    }
}

/// Косинус угла между мостиком и настилом, при котором мостик идёт вдоль
/// него: звенья почти параллельны, и щель между ними — щель, а не
/// перекрёсток двух дорог.
pub(super) const FOOTBRIDGE_ALONG: f32 = 0.9;

/// Два направления идут вдоль друг друга — косинус угла между ними не
/// меньше [`FOOTBRIDGE_ALONG`].
pub(super) fn runs_along(a: Vec2, b: Vec2) -> bool {
    a.dot(b).abs() >= FOOTBRIDGE_ALONG
}

/// Длина клина, которым дорожка расширяется в свой мостик, м
/// ([`push_path_flare`]).
const PATH_SEAM_FLARE: f32 = 3.0;

/// Кромка бортика мостика (`bridge` + `RoadClass::Alley`) на каждом торце
/// наземной дорожки `road`, если мостик продолжает её в узле торца, — от оси,
/// м (`RoadLine::curb_reach`).
pub(super) fn footbridge_reach(
    roads: &[&RoadLine],
    nodes: &RoadNodes,
    road: usize,
) -> [Option<f32>; 2] {
    let own = roads[road];
    let (Some(&first), Some(&last)) = (own.points.first(), own.points.last()) else {
        return [None; 2];
    };
    if own.bridge {
        return [None; 2];
    }
    [first, last].map(|end| {
        nodes
            .roads_at(end)
            .iter()
            .map(|&other| roads[other])
            .filter(|foot| foot.bridge && foot.class == RoadClass::Alley)
            .map(RoadLine::curb_reach)
            .reduce(f32::max)
    })
}

/// Клин, которым дорожка по `points` у торца `end` расширяется от своей
/// полуширины к кромке бортика мостика (`halves`) на последних
/// [`PATH_SEAM_FLARE`] м, и губа под мостик на [`BAND_SEAM_OVERLAP`]: мостик
/// в 3.5 м с бортиками по 0.8 м шире дорожки в 2–3 м, и её прямой торец стоял
/// у головы уступом внутри его бортика (S5, Тула, Одоевский путепровод).
pub(super) fn push_path_flare(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    end: usize,
    [own, reach]: [f32; 2],
    color: LinearRgba,
) {
    let total = polyline_length(points);
    if points.len() < 2 || total <= 0.0 {
        return;
    }
    let length = PATH_SEAM_FLARE.min(total / 2.0);
    let path = if end == 0 {
        let mut path = tapers::cut(points, 0.0, length);
        path.reverse();
        path
    } else {
        tapers::cut(points, total - length, total)
    };
    // по `path` — к торцу: полуширина растёт от своей к кромке бортика
    let Some(out) = path
        .last()
        .zip(path.len().checked_sub(2).and_then(|index| path.get(index)))
        .and_then(|(tip, inner)| (*tip - *inner).try_normalize())
    else {
        return;
    };
    let (along, span) = crate::map::along::arclengths(&path);
    let miters = miter_offsets(&path, false, 1.0);
    let half = |at: f32| own + (reach - own) * at / span.max(f32::EPSILON);
    let tip = path[path.len() - 1];
    let lip = out * BAND_SEAM_OVERLAP;
    let left: Vec<Vec2> = path
        .iter()
        .zip(&miters)
        .zip(&along)
        .map(|((point, miter), &at)| *point + *miter * half(at))
        .collect();
    let mut outline = left;
    outline.push(tip + out.perp() * reach + lip);
    outline.push(tip - out.perp() * reach + lip);
    outline.extend(
        path.iter()
            .zip(&miters)
            .zip(&along)
            .rev()
            .map(|((point, miter), &at)| *point - *miter * half(at)),
    );
    builder.push_polygon(&outline, &[], color);
}
