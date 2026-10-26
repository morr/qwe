//! Мостик (`bridge=yes` на дорожке класса Alley) и его «вдоль» настила:
//! предикат, ручка [`FOOTBRIDGE_ALONG`] и тест [`runs_along`].
//!
//! «Идёт вдоль» — одно понятие с двумя читателями: `axis.rs` выносит по
//! нему мостик, замапленный внутри настила, к кромке настила
//! (`place_bridge_sidewalks`), `aprons.rs` мостит щель между настилом и
//! мостиком рядом (`slit_aprons`).

use bevy::prelude::*;

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

/// Косинус угла между мостиком и настилом, при котором мостик идёт вдоль
/// него: звенья почти параллельны, и щель между ними — щель, а не
/// перекрёсток двух дорог.
pub(super) const FOOTBRIDGE_ALONG: f32 = 0.9;

/// Два направления идут вдоль друг друга — косинус угла между ними не
/// меньше [`FOOTBRIDGE_ALONG`].
pub(super) fn runs_along(a: Vec2, b: Vec2) -> bool {
    a.dot(b).abs() >= FOOTBRIDGE_ALONG
}
