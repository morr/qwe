//! Карта из OpenStreetMap: модель, Overpass-выгрузка, парсинг в `MapData`.

pub mod model;
pub mod overpass;
pub mod parse;

pub use self::model::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
