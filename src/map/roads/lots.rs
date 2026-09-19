//! Дороги на большой стоянке — слоями **поверх** её асфальта.
//!
//! Стоянка лежит выше дорожных лент (`Z_PARKING` над `Z_ROAD`) и кроет всё,
//! что на неё заходит. Для двора это верно: асфальт площадки и есть проезд. У
//! большой стоянки ([`parking::is_ground`]) сквозь площадку идёт настоящая
//! дорога ([`parking::is_through`]) — у ТРЦ «Макси» бульвар с односторонним
//! движением и тремя кольцами, — и спрятанная, она оставляла восемь гектаров
//! штриховки без единого ориентира (отчёт автора).
//!
//! Поэтому кусок каждой улицы, лежащий **внутри** контура большой стоянки,
//! кладётся второй раз, выше её асфальта: сквозная дорога — с бордюром
//! ([`parking::kerb_width`], слой `lot_sidewalks`), всё остальное, включая
//! проезды рядов, — просто асфальтом (`lot_roads`). Проезды здесь не ради
//! самих себя — их асфальт того же тона, что у площадки, и на ней не виден, —
//! а ради **устьев**: лёгший поверх бордюра, проезд прорезает в нём въезд, и
//! бордюр вдоль бульвара выходит островками между рядами, как он и стоит.

use bevy::prelude::*;

use super::{ROAD_COLOR, RoadJoin, RoadStyle, SIDEWALK_COLOR, push_ribbon};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::model::{MapData, PolyArea, RoadLine, point_in_area, ring_bounds};
use crate::map::parking::{is_ground, is_through, kerb_width};

/// Шаг, которым ось дороги ощупывается на «внутри ли площадки», м.
const PROBE_STEP: f32 = 2.0;
/// Сколько раз делится пополам шаг, на котором ось пересекла контур: 2 м / 2⁷ —
/// полтора сантиметра.
const BISECTIONS: usize = 7;

/// Большие стоянки карты с габаритами — чтобы дорога в другом конце города не
/// ощупывала их контуры.
pub(super) struct Grounds<'a>(Vec<(&'a PolyArea, Vec2, Vec2)>);

/// Кусок оси внутри площадки; `cut` — обрезан ли он контуром с этого конца
/// (иначе там кончается сама дорога).
pub(super) struct Run {
    pub points: Vec<Vec2>,
    pub cut: [bool; 2],
}

impl<'a> Grounds<'a> {
    pub fn of(map: &'a MapData) -> Self {
        Self(
            map.parking
                .iter()
                .filter(|lot| is_ground(lot))
                .map(|lot| {
                    let (low, high) = ring_bounds(&lot.outer);
                    (lot, low, high)
                })
                .collect(),
        )
    }

    /// Куски ломаной `path`, лежащие внутри больших стоянок.
    pub fn runs(&self, path: &[Vec2]) -> Vec<Run> {
        if self.0.is_empty() || path.len() < 2 {
            return Vec::new();
        }
        let (low, high) = ring_bounds(path);
        self.0
            .iter()
            .filter(|(_, lot_low, lot_high)| {
                low.cmple(*lot_high).all() && high.cmpge(*lot_low).all()
            })
            .flat_map(|(lot, _, _)| runs_inside(path, lot))
            .collect()
    }
}

fn runs_inside(path: &[Vec2], lot: &PolyArea) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut current: Option<Run> = None;
    let mut inside = point_in_area(path[0], lot);
    if inside {
        current = Some(Run {
            points: vec![path[0]],
            cut: [false, false],
        });
    }
    for pair in path.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let steps = (from.distance(to) / PROBE_STEP).ceil().max(1.0) as usize;
        let mut previous = from;
        for step in 1..=steps {
            let at = from.lerp(to, step as f32 / steps as f32);
            let now = point_in_area(at, lot);
            if now != inside {
                let edge = crossing(previous, at, inside, lot);
                match current.take() {
                    Some(mut run) => {
                        run.points.push(edge);
                        run.cut[1] = true;
                        runs.push(run);
                    }
                    None => {
                        current = Some(Run {
                            points: vec![edge],
                            cut: [true, false],
                        });
                    }
                }
                inside = now;
            }
            previous = at;
        }
        if let Some(run) = &mut current {
            run.points.push(to);
        }
    }
    runs.extend(current);
    runs.retain(|run| run.points.len() >= 2);
    runs
}

/// Точка, в которой отрезок `from` → `to` пересёк контур, — делением пополам;
/// `inside` — лежит ли внутри `from`.
fn crossing(mut from: Vec2, mut to: Vec2, inside: bool, lot: &PolyArea) -> Vec2 {
    for _ in 0..BISECTIONS {
        let middle = from.midpoint(to);
        if point_in_area(middle, lot) == inside {
            from = middle;
        } else {
            to = middle;
        }
    }
    from.midpoint(to)
}

/// Два слоя поверх асфальта стоянки: бордюры сквозных дорог и сам асфальт.
pub(super) struct LotLayers {
    pub sidewalks: MeshBuilder,
    pub roads: MeshBuilder,
}

impl LotLayers {
    pub fn new() -> Self {
        Self {
            sidewalks: MeshBuilder::with_surface_coords(),
            roads: MeshBuilder::with_surface_coords(),
        }
    }

    /// Кусок дороги на стоянке. Разметки у него нет: сквозь стоянки идут
    /// проезды, которым она и не положена, а проезжая часть с полосами,
    /// зашедшая на площадку, теряет их на ней — цена одного слоя на всё.
    pub fn push(&mut self, road: &RoadLine, run: &Run, style: &RoadStyle) {
        if style.sidewalks && is_through(road) {
            let width = road.width + 2.0 * kerb_width(road);
            let color = SIDEWALK_COLOR.to_linear();
            match style.join {
                RoadJoin::Square => self.sidewalks.push_polyline(&run.points, width, color),
                // у контура площадки бордюр обрезан встык: скруглённый торец
                // вылез бы за неё светлой скобкой вокруг уходящей дороги
                _ => self.sidewalks.push_ribbon_capped(
                    &run.points,
                    false,
                    width,
                    color,
                    RibbonJoin::Round,
                    run.cut.map(|cut| {
                        if cut {
                            RibbonCap::Butt
                        } else {
                            RibbonCap::Round
                        }
                    }),
                ),
            }
        }
        push_ribbon(
            &mut self.roads,
            &run.points,
            road.width,
            ROAD_COLOR.to_linear(),
            style.join,
        );
    }
}
