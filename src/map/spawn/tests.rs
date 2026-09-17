//! Слои поверхностей, собранные без мира.
//!
//! Эти тесты появились вместе со швом `mesh_surfaces`: до него тринадцать
//! слоёв города собирались прямо в теле системы `spawn_map`, и единственной
//! дверью в них был поднятый мир Bevy — ни лестницу z, ни выбор материала,
//! ни счётчик вырожденных контуров проверить было нечем.

use super::*;
use crate::map::osm::fixture;

/// Пустая карта: все тринадцать слоёв описаны и все пусты, кроме земли.
fn empty() -> (Vec<LayerMesh>, SurfaceReport) {
    mesh_surfaces(&MapData::default(), &parking::ParkingLayout::default())
}

fn layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a LayerMesh {
    layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("слоя {name} нет"))
}

/// Тринадцать слоёв под своими именами — имя то самое, под которым слой ищут в
/// живом мире через BRP.
///
/// Порядок **в списке** ни на что не влияет: каждый слой уходит своей
/// сущностью со своим z, и список идёт не по возрастанию (площадка 2.003 стоит
/// перед стоянкой 2.001, а краска дописана в хвост). Что обязано держаться —
/// краска **над** своим покрытием: иначе асфальт накрывает разметку мест,
/// которая по нему и нарисована.
#[test]
fn every_layer_has_its_own_name_and_the_paint_lies_over_its_surface() {
    let (layers, _) = empty();
    assert_eq!(layers.len(), 13);
    let names: std::collections::HashSet<&str> = layers.iter().map(|layer| layer.name).collect();
    assert_eq!(names.len(), layers.len(), "имя слоя повторяется");

    for (surface, paint) in [("parking", "parking_lines"), ("pitches", "pitch_lines")] {
        assert!(
            layer(&layers, paint).z > layer(&layers, surface).z,
            "{paint} под {surface}"
        );
    }
}

/// Земля — квад на всю карту, и она есть всегда; всё остальное на пустой карте
/// пусто, так что пустой город не спавнит двенадцати пустых сущностей
/// (`spawn_layer` отбрасывает пустой сборщик).
#[test]
fn an_empty_city_draws_nothing_but_the_ground() {
    let (layers, report) = empty();
    for layer in &layers {
        let empty = layer.builder.is_empty();
        assert_eq!(empty, layer.name != "ground", "{}", layer.name);
    }
    assert_eq!(report.skipped, 0);
    assert!(report.vertices > 0);
}

/// Краска — плоским материалом, покрытие — фактурным. Разметка мест и полей это
/// белая краска по асфальту и по корту, а не фактура покрытия, и общий
/// `SurfaceMaterial` положил бы на неё свой шум.
#[test]
fn the_paint_is_flat_and_every_surface_is_textured() {
    let (layers, _) = empty();
    for layer in &layers {
        let paint = matches!(layer.name, "pitch_lines" | "parking_lines");
        let flat = layer.material == MaterialSpec::Flat;
        assert_eq!(flat, paint, "{}", layer.name);
    }
}

/// Зелень попадает каждая в свой слой, а не в общий: у двора, парка, леса,
/// луга и песка разные `SurfaceKind`, и одна заливка на всех означала бы траву
/// на бетонной площадке.
#[test]
fn each_green_area_reaches_its_own_layer() {
    let map = MapData {
        parks: vec![fixture::area(
            AreaKind::Park,
            fixture::square(Vec2::ZERO, 50.0),
        )],
        woods: vec![fixture::wood(fixture::square(Vec2::new(200.0, 0.0), 50.0))],
        grass: vec![fixture::area(
            AreaKind::Grass,
            fixture::square(Vec2::new(400.0, 0.0), 50.0),
        )],
        sand: vec![fixture::area(
            AreaKind::Sand,
            fixture::square(Vec2::new(600.0, 0.0), 50.0),
        )],
        ..default()
    };
    let (layers, report) = mesh_surfaces(&map, &parking::ParkingLayout::default());

    for name in ["parks", "woods", "grass", "sand"] {
        assert!(!layer(&layers, name).builder.is_empty(), "{name} пуст");
    }
    assert!(layer(&layers, "landuse_yards").builder.is_empty());
    assert_eq!(report.skipped, 0);
}

/// Жилой квартал и промзона — два слоя, и разводит их вид застройки, а не
/// цвет: фактура двора и фактура утоптанной промплощадки разные по смыслу.
#[test]
fn the_landuse_block_splits_into_a_yard_and_a_works() {
    let map = MapData {
        landuse: vec![
            fixture::area(AreaKind::Residential, fixture::square(Vec2::ZERO, 50.0)),
            fixture::area(
                AreaKind::Industrial,
                fixture::square(Vec2::new(200.0, 0.0), 50.0),
            ),
        ],
        ..default()
    };
    let (layers, _) = mesh_surfaces(&map, &parking::ParkingLayout::default());

    assert!(!layer(&layers, "landuse_yards").builder.is_empty());
    assert!(!layer(&layers, "landuse_works").builder.is_empty());
    assert_eq!(
        layer(&layers, "landuse_yards").material,
        MaterialSpec::Surface(SurfaceKind::Yard)
    );
    assert_eq!(
        layer(&layers, "landuse_works").material,
        MaterialSpec::Surface(SurfaceKind::Ground)
    );
}

/// Вырожденный контур не рисуется, но и не теряется: он считается, и ненулевой
/// счётчик — повод смотреть в парс, а не в сборку. До шва это число уходило в
/// `warn!` и ни одному тесту не было видно.
#[test]
fn a_degenerate_contour_is_counted_rather_than_drawn() {
    let line = vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(20.0, 0.0)];
    let map = MapData {
        parks: vec![fixture::area(AreaKind::Park, line)],
        ..default()
    };
    let (layers, report) = mesh_surfaces(&map, &parking::ParkingLayout::default());

    assert!(layer(&layers, "parks").builder.is_empty());
    assert_eq!(report.skipped, 1);
}
