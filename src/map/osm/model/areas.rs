//! Площадные слои карты — перечень видов: какое поле [`MapData`] держит
//! полигоны слоя, где слой отвечает в снимке контуров и в пробе точки и чем
//! подписывается находка. Один источник списка видов: снимок контуров
//! ([`super::contours`]), индекс пробы (`crate::map::inspect`) и сборка
//! поверхностей (`crate::map::spawn`) берут из него и состав слоёв, и их
//! поля. Новый вид площадной фичи — поле в `MapData` и строка в перечне
//! ([`AreaLayer::ALL`]).

use bevy::prelude::*;

use super::{MapData, PolyArea};
use crate::settings::{
    Z_BUILDING, Z_GRASS, Z_LANDUSE, Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PITCH, Z_POND, Z_SAND,
    Z_WOOD,
};

/// Вид площадного слоя — одно из полей `Vec<PolyArea>` в [`MapData`]. Вектор
/// говорит, в каком **слое** полигон нарисован, а [`super::AreaKind`] в нём —
/// какой он **член** этого слоя; перечнем видов здесь вектор перестаёт быть
/// списком, размноженным по потребителям.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AreaLayer {
    /// Здания и крепостные стены-полигоны — [`MapData::buildings`]; мешит не
    /// сборка поверхностей, а `map/buildings/`.
    Building,
    /// Площадная вода — [`MapData::water`].
    Water,
    /// Парки — [`MapData::parks`].
    Park,
    /// Лесные массивы — [`MapData::woods`]; только здесь растут деревья.
    Wood,
    /// Луга и газоны — [`MapData::grass`].
    Grass,
    /// Песчаные пляжи — [`MapData::sand`].
    Sand,
    /// Кварталы `landuse` — [`MapData::landuse`].
    Landuse,
    /// Карманы земли — [`MapData::pockets`]; сеются доводочным проходом
    /// (`parse/pockets.rs`), рисуются слоями кварталов и газона.
    Pocket,
    /// Стоянки — [`MapData::parking`].
    Parking,
    /// Площадки — [`MapData::pitches`].
    Pitch,
}

/// Дескриптор вида: его поле, участие в снимке контуров и в индексе пробы,
/// слой и подпись находки пробы. Стилизация отрисовки (цвета, каймы, разрезы
/// слоёв) остаётся в потребителях — здесь только то, что верно для вида
/// целиком.
#[derive(Clone, Copy)]
pub struct AreaLayerDesc {
    pub layer: AreaLayer,
    /// Поле [`MapData`] с полигонами слоя — единственное место, где имя поля
    /// стоит рядом со списком видов.
    pub field: fn(&MapData) -> &[PolyArea],
    /// Кольца слоя доходят до снимка контуров OSM (`OsmContours::of`).
    /// У [`AreaLayer::Pocket`] фальшь, и это не дрейф, а порядок конвейера:
    /// снимок берётся в конце элементного цикла, а карманы сеются доводочным
    /// проходом позже — поле на момент снимка пусто всегда.
    pub contours: bool,
    /// Полигоны слоя отвечают пробе точки (`InspectIndex::prebuild`).
    pub index: bool,
    /// Слой находки (z, где слой рисуется) и её подпись — от вида полигона:
    /// у кварталов и карманов вид решает и то и другое, у стоянки вид чужой
    /// полигона означает «не находка». `None` — находки нет.
    pub hit: fn(&PolyArea) -> Option<(f32, String)>,
}

impl AreaLayer {
    /// Перечень всех видов площадных слоёв — единственный источник списка.
    /// Порядок строк = порядок вариантов = порядок полей в `MapData`; снимок
    /// контуров ведёт кольца именно в нём.
    pub const ALL: &'static [AreaLayerDesc] = &[
        AreaLayerDesc {
            layer: AreaLayer::Building,
            field: |map: &MapData| &map.buildings,
            contours: true,
            index: true,
            hit: hit_building,
        },
        AreaLayerDesc {
            layer: AreaLayer::Water,
            field: |map: &MapData| &map.water,
            contours: true,
            index: true,
            hit: hit_water,
        },
        AreaLayerDesc {
            layer: AreaLayer::Park,
            field: |map: &MapData| &map.parks,
            contours: true,
            index: true,
            hit: hit_park,
        },
        AreaLayerDesc {
            layer: AreaLayer::Wood,
            field: |map: &MapData| &map.woods,
            contours: true,
            index: true,
            hit: hit_wood,
        },
        AreaLayerDesc {
            layer: AreaLayer::Grass,
            field: |map: &MapData| &map.grass,
            contours: true,
            index: true,
            hit: hit_grass,
        },
        AreaLayerDesc {
            layer: AreaLayer::Sand,
            field: |map: &MapData| &map.sand,
            contours: true,
            index: true,
            hit: hit_sand,
        },
        AreaLayerDesc {
            layer: AreaLayer::Landuse,
            field: |map: &MapData| &map.landuse,
            contours: true,
            index: true,
            hit: hit_landuse,
        },
        AreaLayerDesc {
            layer: AreaLayer::Pocket,
            field: |map: &MapData| &map.pockets,
            // снимок берётся раньше, чем карманы сеются, — см. поле `contours`
            contours: false,
            index: true,
            hit: hit_pocket,
        },
        AreaLayerDesc {
            layer: AreaLayer::Parking,
            field: |map: &MapData| &map.parking,
            contours: true,
            index: true,
            hit: hit_parking,
        },
        AreaLayerDesc {
            layer: AreaLayer::Pitch,
            field: |map: &MapData| &map.pitches,
            contours: true,
            index: true,
            hit: hit_pitch,
        },
    ];

    /// Дескриптор вида — строка перечня.
    pub fn desc(self) -> &'static AreaLayerDesc {
        // индекс = порядок вариантов; тест ниже держит перечень в том же порядке
        &Self::ALL[self as usize]
    }

    /// Полигоны слоя — поле [`MapData`] из дескриптора.
    pub fn field(self, map: &MapData) -> &[PolyArea] {
        (self.desc().field)(map)
    }
}

/// Находка здания: назначение и высота.
fn hit_building(area: &PolyArea) -> Option<(f32, String)> {
    Some((Z_BUILDING, building_label(area)))
}

/// Слои с постоянным слоем и подписью: вода, парк, лес, луг, песок.
fn hit_water(_: &PolyArea) -> Option<(f32, String)> {
    Some((Z_POND, "water".to_string()))
}
fn hit_park(_: &PolyArea) -> Option<(f32, String)> {
    Some((Z_PARK, "park".to_string()))
}
fn hit_wood(_: &PolyArea) -> Option<(f32, String)> {
    Some((Z_WOOD, "wood".to_string()))
}
fn hit_grass(_: &PolyArea) -> Option<(f32, String)> {
    Some((Z_GRASS, "grass".to_string()))
}
fn hit_sand(_: &PolyArea) -> Option<(f32, String)> {
    Some((Z_SAND, "sand".to_string()))
}

/// Находка квартала: промзона лежит своим слоем, всё прочее — двором.
fn hit_landuse(area: &PolyArea) -> Option<(f32, String)> {
    let z = match area.kind {
        super::AreaKind::Industrial => Z_LANDUSE,
        _ => Z_LANDUSE_YARD,
    };
    Some((z, format!("landuse {}", lower(area.kind))))
}

/// Находка кармана: карман-луг лежит в слое газона, прочие — двором.
fn hit_pocket(area: &PolyArea) -> Option<(f32, String)> {
    let z = match area.kind {
        super::AreaKind::Grass => Z_GRASS,
        _ => Z_LANDUSE_YARD,
    };
    Some((z, format!("pocket {}", lower(area.kind))))
}

/// Находка стоянки: вид чужой полигона — не находка.
fn hit_parking(area: &PolyArea) -> Option<(f32, String)> {
    let super::AreaKind::Parking(kind) = area.kind else {
        return None;
    };
    Some((Z_PARKING, format!("parking lot {}", lower(kind))))
}

/// Находка площадки: покрытие подписывает вид спорта.
fn hit_pitch(area: &PolyArea) -> Option<(f32, String)> {
    Some((Z_PITCH, pitch_label(area.kind)))
}

/// Имя варианта в нижнем регистре — для подписей (`Residential` →
/// `residential`).
pub(crate) fn lower<T: std::fmt::Debug>(value: T) -> String {
    format!("{value:?}").to_lowercase()
}

/// Назначение здания для подписи: составные имена читаемее слитного Debug
/// (`garageblock` → `garage block`); у храма в значении вера — в подпись она
/// не нужна.
fn building_use_label(use_: super::BuildingUse) -> &'static str {
    match use_ {
        super::BuildingUse::House => "house",
        super::BuildingUse::Apartments => "apartments",
        super::BuildingUse::Commercial => "commercial",
        super::BuildingUse::Retail => "retail",
        super::BuildingUse::Industrial => "industrial",
        super::BuildingUse::Garage => "garage",
        super::BuildingUse::GarageBlock => "garage block",
        super::BuildingUse::Church(_) => "church",
        super::BuildingUse::Public => "public",
        super::BuildingUse::Other => "other",
    }
}

/// Подпись здания: назначение по `building=*` и высота, если тег был.
fn building_label(area: &PolyArea) -> String {
    let mut label = match area.kind {
        super::AreaKind::Kremlin => "kremlin building".to_string(),
        _ => match area.building_use {
            super::BuildingUse::Other => "building".to_string(),
            use_ => format!("building {}", building_use_label(use_)),
        },
    };
    if let Some(height) = area.height {
        label.push_str(&format!(", {:.0} m", height));
    }
    label
}

/// Подпись площадки: вид спорта читаемее номера варианта.
fn pitch_label(kind: super::AreaKind) -> String {
    let super::AreaKind::Pitch(kind) = kind else {
        return format!("pitch {}", lower(kind));
    };
    match kind {
        super::PitchKind::Soccer => "soccer field".to_string(),
        super::PitchKind::Hard => "hard court".to_string(),
        super::PitchKind::Track => "running track".to_string(),
        super::PitchKind::Playground => "playground".to_string(),
        super::PitchKind::Ground => "sports ground".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// По одной различимой зоне (свой `osm_id`) в каждом поле: дескриптор вида
    /// обязан видеть её в своём поле и ни в каком другом — строка перечня,
    /// показывающая не в то поле, ломает и снимок контуров, и пробу.
    #[test]
    fn every_layer_descriptor_points_at_its_own_field() {
        let mut map = MapData::default();
        let mut id = 0;
        macro_rules! one_area {
            ($($field:ident),* $(,)?) => {$(
                id += 1;
                map.$field.push(PolyArea {
                    outer: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
                    holes: Vec::new(),
                    kind: super::super::AreaKind::Grass,
                    building_use: Default::default(),
                    height: None,
                    storeys: None,
                    entrances: Vec::new(),
                    colours: Default::default(),
                    osm_id: Some(id),
                });
            )*};
        }
        one_area!(
            buildings, water, parks, woods, grass, sand, landuse, pockets, parking, pitches,
        );

        for desc in AreaLayer::ALL {
            let areas = desc.field(&map);
            assert_eq!(areas.len(), 1, "{:?}: в поле не одна зона", desc.layer);
            assert_eq!(
                areas[0].osm_id,
                Some(id_of(desc.layer)),
                "{:?}: дескриптор показывает не в своё поле",
                desc.layer
            );
        }
    }

    /// Чей `osm_id` у зоны вида — зона по счёту вида в перечне, считая с 1.
    fn id_of(layer: AreaLayer) -> u64 {
        (layer as usize + 1) as u64
    }

    /// Перечень идёт в порядке вариантов: `desc()` берёт строку по индексу
    /// варианта.
    #[test]
    fn the_descriptor_list_follows_the_variant_order() {
        for (i, desc) in AreaLayer::ALL.iter().enumerate() {
            assert_eq!(desc.layer as usize, i, "строка {i} не своего варианта");
        }
    }

    /// Карманы не участвуют в снимке контуров, и это решение перечня, а не
    /// дрейф: снимок берётся до доводочных проходов, когда карманов ещё нет
    /// (`parse/pockets.rs`).
    #[test]
    fn pockets_are_not_in_the_contours_snapshot() {
        assert!(!AreaLayer::Pocket.desc().contours);
        for desc in AreaLayer::ALL {
            if desc.layer != AreaLayer::Pocket {
                assert!(desc.contours, "{:?}: не в снимке контуров", desc.layer);
            }
        }
    }
}
