//! Подписи пробы: чем подписана находка каждого признака — класс `highway`,
//! назначение здания, вид площадки, краска узла — и как находки печатаются
//! строкой для панели инспектора и отчёта `brp probe`. Находка дороги
//! (`road_hit`) — тоже здесь: полотно и дорисованные полосы, с расстоянием до
//! осевой и стороной.

use bevy::prelude::*;

use crate::map::RoadStyle;
use crate::map::osm::model::{
    AreaKind, BuildingUse, Highway, PitchKind, PolyArea, RoadClass, RoadLine, RoadNodeKind,
    SidewalkSide, StructureKind, closest_on_segment,
};
use crate::map::roads::PieceKind;
use crate::settings::{Z_ALLEY, Z_BRIDGE, Z_ROAD, Z_SIDEWALK, Z_UNPAVED_ROAD};

use super::{FeatureRef, Hit, HitReason, HitSource};

/// Одна находка строкой `z  label` — колонка z и отступ общие у обоих
/// выходов индекса: панели ([`format_hits`]) и отчёта пробы (`dev.rs`),
/// чтобы один и тот же признак читался в них одинаково. У находки с OSM id
/// элемента-источника подпись ведёт хвостом `#id` — теги элемента показывает
/// `tools/osm_near --id <id>`.
pub fn hit_line(hit: &Hit) -> String {
    match hit.id {
        Some(id) => format!("z {:6.3}  {} #{id}", hit.z, hit.label),
        None => format!("z {:6.3}  {}", hit.z, hit.label),
    }
}

/// Захваченная находка (клавиша `\` панели инспектора) строкой: колонки те же,
/// что у [`hit_line`], но ведёт `c` вместо `z` — `tools/shot_coords` собирает
/// под строкой камеры только `z …` строки, так что захват в его ответе с
/// находками под курсором не смешается и в конкурсе выбранной строки не
/// участвует.
pub fn captured_line(hit: &Hit) -> String {
    match hit.id {
        Some(id) => format!("c {:6.3}  {} #{id}", hit.z, hit.label),
        None => format!("c {:6.3}  {}", hit.z, hit.label),
    }
}

/// Строки пробы для панели инспектора и отчёта `brp probe`: одна находка —
/// строка `z  label`, самый верхний слой первым. У находки с OSM id
/// элемента-источника подпись ведёт хвостом `#id` — теги элемента показывает
/// `tools/osm_near --id <id>`. Строк больше `cap` — хвост `+N more`.
pub fn format_hits(hits: &[Hit], cap: usize) -> String {
    let mut out = String::new();
    for hit in hits.iter().take(cap) {
        out.push_str(&hit_line(hit));
        out.push('\n');
    }
    if hits.len() > cap {
        out.push_str(&format!("+{} more\n", hits.len() - cap));
    }
    out.trim_end().to_string()
}

/// Имя варианта в нижнем регистре — для подписей (`Residential` →
/// `residential`).
pub(super) fn lower<T: std::fmt::Debug>(value: T) -> String {
    format!("{value:?}").to_lowercase()
}

/// Находка дороги в точке: полотно, а за ним полоса тротуара, которую дорога
/// дорисовывает. Полоса мерится от OSM-осевой по закону ширины модели
/// ([`RoadLine::sidewalk`]); сторона — по знаку к ближайшему отрезку осевой.
/// Ручка [`RoadStyle::sidewalks`] убирает полосу, как и у рисунка
/// (`Drawn::verges_drawn`). Состав обочины за тротуаром отвечают
/// отрисованные куски (`FeatureRef::Piece`), не дорога.
pub(super) fn road_hit(
    road_index: u32,
    road: &RoadLine,
    style: &RoadStyle,
    at: Vec2,
) -> Option<Hit> {
    let (dist, left) = distance_and_side(at, &road.points);
    let half = road.width / 2.0;
    if dist <= half {
        return Some(Hit {
            z: road_z(road),
            label: road_label(road),
            src: HitSource::Osm,
            id: road.osm_id,
            feature: FeatureRef::Road(road_index),
        });
    }
    if !style.sidewalks {
        return None;
    }
    let profile = road.sidewalk();
    let band = profile.band()?;
    let side = usize::from(!left);
    if profile.on(side).is_some() && dist <= half + band {
        // on(side) — Some, значит сторона либо Tagged, либо Inferred
        let reason = if road.sidewalks[side] == SidewalkSide::Tagged {
            HitReason::Tagged
        } else {
            HitReason::Inferred
        };
        return Some(Hit {
            z: Z_SIDEWALK,
            label: format!("sidewalk (auto, {})", reason.as_str()),
            src: HitSource::Generated(reason),
            id: road.osm_id,
            feature: FeatureRef::SidewalkBand {
                road: road_index,
                side: side as u8,
            },
        });
    }
    None
}

/// Подпись отрисованного куска дороги: состав обочины, лоскут или кусок
/// узла — то, что проход сборки дорог рисует поверх модели.
pub(super) fn piece_label(kind: PieceKind) -> String {
    match kind {
        PieceKind::VergePaving => "verge paving (auto)",
        PieceKind::VergeLawn => "verge lawn (auto)",
        PieceKind::VergeYard => "verge yard (auto)",
        PieceKind::PavingScrap => "paving scrap (auto)",
        PieceKind::RoadCorner => "road corner (auto)",
        PieceKind::SidewalkCorner => "sidewalk corner (auto)",
    }
    .to_string()
}

/// Расстояние от точки до ломаной и сторона ближайшего отрезка: `false` —
/// слева по ходу точек (сторона 0 модели), `true` — справа. Один проход
/// вместо двух: расстояние — то же, что у `distance_to_polyline`, сторона —
/// у отрезка, который его дал. Смещение меряется от отрезка к точке, и
/// `perp` смотрит влево по ходу — точка слева даёт плюс (та же сторона 0,
/// что у `parse/verges.rs`, там смещение развёрнуто). Равные отрезки —
/// к первому.
fn distance_and_side(point: Vec2, points: &[Vec2]) -> (f32, bool) {
    let mut best = (f32::INFINITY, false);
    for segment in points.windows(2) {
        let closest = closest_on_segment(point, segment[0], segment[1]);
        let dist = closest.distance(point);
        if dist < best.0 {
            let direction = (segment[1] - segment[0]).normalize_or_zero();
            let left = (point - closest).dot(direction.perp()) >= 0.0;
            best = (dist, left);
        }
    }
    best
}

/// Слой дороги: настил улицы, аллеи или грунтовой улицы, у моста — мостовой.
fn road_z(road: &RoadLine) -> f32 {
    if road.bridge {
        return Z_BRIDGE;
    }
    match road.class {
        RoadClass::Alley => Z_ALLEY,
        RoadClass::Street if road.is_unpaved_street() => Z_UNPAVED_ROAD,
        RoadClass::Street => Z_ROAD,
    }
}

/// Подпись дороги: класс `highway` и ширина — то, по чему признак ищут в
/// данных; мост и тротуары — флагами. Полоса тротуара под точкой отвечает
/// отдельной находкой (`sidewalk (auto, …)`) — по закону ширины модели;
/// точная нарисованная геометрия остаётся делом сборки дорог.
fn road_label(road: &RoadLine) -> String {
    let mut label = match road.class {
        RoadClass::Street => {
            format!("road {} {:.1} m", highway_label(road.highway), road.width)
        }
        RoadClass::Alley => format!("path {} {:.1} m", highway_label(road.highway), road.width),
    };
    if road.bridge {
        label.push_str(", bridge");
    }
    if road
        .sidewalks
        .iter()
        .any(|side| *side != SidewalkSide::None)
    {
        label.push_str(", sidewalks");
    }
    label
}

/// Имя класса `highway` для подписи: связки — через пробел, иначе Debug
/// слепил бы `motorwaylink`.
fn highway_label(highway: Highway) -> &'static str {
    match highway {
        Highway::Motorway => "motorway",
        Highway::Trunk => "trunk",
        Highway::Primary => "primary",
        Highway::Secondary => "secondary",
        Highway::Tertiary => "tertiary",
        Highway::MotorwayLink => "motorway link",
        Highway::TrunkLink => "trunk link",
        Highway::PrimaryLink => "primary link",
        Highway::SecondaryLink => "secondary link",
        Highway::TertiaryLink => "tertiary link",
        Highway::Residential => "residential",
        Highway::Unclassified => "unclassified",
        Highway::LivingStreet => "living street",
        Highway::Service => "service",
        Highway::Path => "path",
    }
}

/// Назначение здания для подписи: составные имена читаемее слитного Debug
/// (`garageblock` → `garage block`); у храма в значении вера — в подпись она
/// не нужна.
fn building_use_label(use_: BuildingUse) -> &'static str {
    match use_ {
        BuildingUse::House => "house",
        BuildingUse::Apartments => "apartments",
        BuildingUse::Commercial => "commercial",
        BuildingUse::Retail => "retail",
        BuildingUse::Industrial => "industrial",
        BuildingUse::Garage => "garage",
        BuildingUse::GarageBlock => "garage block",
        BuildingUse::Church(_) => "church",
        BuildingUse::Public => "public",
        BuildingUse::Other => "other",
    }
}

/// Подпись здания: назначение по `building=*` и высота, если тег был.
pub(super) fn building_label(area: &PolyArea) -> String {
    let mut label = match area.kind {
        AreaKind::Kremlin => "kremlin building".to_string(),
        _ => match area.building_use {
            BuildingUse::Other => "building".to_string(),
            use_ => format!("building {}", building_use_label(use_)),
        },
    };
    if let Some(height) = area.height {
        label.push_str(&format!(", {:.0} m", height));
    }
    label
}

/// Подпись площадки: вид спорта читаемее номера варианта.
pub(super) fn pitch_label(kind: AreaKind) -> String {
    let AreaKind::Pitch(kind) = kind else {
        return format!("pitch {}", lower(kind));
    };
    match kind {
        PitchKind::Soccer => "soccer field".to_string(),
        PitchKind::Hard => "hard court".to_string(),
        PitchKind::Track => "running track".to_string(),
        PitchKind::Playground => "playground".to_string(),
        PitchKind::Ground => "sports ground".to_string(),
    }
}

/// Подпись дорожного узла — краска, которую он кладёт на асфальт.
pub(super) fn node_label(kind: RoadNodeKind) -> String {
    match kind {
        RoadNodeKind::Crossing {
            signals, island, ..
        } => {
            let mut label = String::from("crossing");
            if signals {
                label.push_str(" (signals)");
            }
            if island {
                label.push_str(" (island)");
            }
            label
        }
        RoadNodeKind::TrafficSignals => "traffic signals".to_string(),
        RoadNodeKind::Stop => "stop sign".to_string(),
        RoadNodeKind::GiveWay => "give-way sign".to_string(),
        RoadNodeKind::MiniRoundabout => "mini roundabout".to_string(),
        RoadNodeKind::TurningCircle => "turning circle".to_string(),
        RoadNodeKind::Island => "traffic island".to_string(),
    }
}

/// Подпись промышленного цилиндра.
pub(super) fn structure_label(kind: StructureKind) -> String {
    match kind {
        StructureKind::Tank => "storage tank".to_string(),
        StructureKind::Silo => "silo".to_string(),
        StructureKind::Chimney => "chimney".to_string(),
        StructureKind::WaterTower => "water tower".to_string(),
        StructureKind::Gasometer => "gasometer".to_string(),
    }
}
