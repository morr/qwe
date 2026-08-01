//! Карта из OpenStreetMap: модель, Overpass-выгрузка, парсинг в `MapData`,
//! досочинение входов и посадка деревьев по разобранной карте.

pub mod download;
pub mod entrances;
pub mod model;
pub mod overpass;
pub mod parse;
mod planting;

pub use self::download::{JobState, MapLoadJob, OVERPASS_MIRRORS, start_load_thread};
pub use self::model::{
    AreaKind, MapData, PolyArea, RailKind, RailLine, RoadClass, RoadLine, RowTrees, TreeCompose,
    TreeRow, TreeRowLayout, TreeRowPlacement, WallLine, WaterKind, WaterLine,
};
// потолок плотности считается от минимального зазора посадки, поэтому живёт
// рядом с ним, а не в `settings.rs`
pub use self::planting::TREE_DENSITY_MAX;
