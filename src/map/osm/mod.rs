//! Карта из OpenStreetMap: модель, Overpass-выгрузка, парсинг в `MapData`.

pub mod download;
pub mod entrances;
pub mod model;
pub mod overpass;
pub mod parse;

pub use self::download::{JobState, MapLoadJob, OVERPASS_MIRRORS, start_load_thread};
pub use self::model::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
