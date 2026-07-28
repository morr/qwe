//! Сборка слитых 2D-мешей слоёв карты: тысячи полигонов OSM в один
//! `Mesh2d` с вершинными цветами (стоковый `ColorMaterial` их умножает).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

#[derive(Default)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    skipped_polygons: usize,
}

impl MeshBuilder {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn skipped_polygons(&self) -> usize {
        self.skipped_polygons
    }

    /// Полигон с дырками через earcut. Вырожденный/кривой — пропуск со
    /// счётчиком, один плохой контур OSM не должен ронять всю карту.
    pub fn push_polygon(&mut self, outer: &[Vec2], holes: &[Vec<Vec2>], color: LinearRgba) {
        if outer.len() < 3 {
            self.skipped_polygons += 1;
            return;
        }

        let mut coordinates: Vec<f64> =
            Vec::with_capacity((outer.len() + holes.iter().map(Vec::len).sum::<usize>()) * 2);
        let mut hole_starts = Vec::with_capacity(holes.len());
        for point in outer {
            coordinates.push(point.x as f64);
            coordinates.push(point.y as f64);
        }
        for hole in holes {
            hole_starts.push(coordinates.len() / 2);
            for point in hole {
                coordinates.push(point.x as f64);
                coordinates.push(point.y as f64);
            }
        }

        let Ok(triangles) = earcutr::earcut(&coordinates, &hole_starts, 2) else {
            self.skipped_polygons += 1;
            return;
        };
        if triangles.is_empty() {
            self.skipped_polygons += 1;
            return;
        }

        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        for chunk in coordinates.chunks_exact(2) {
            self.positions.push([chunk[0] as f32, chunk[1] as f32, 0.0]);
            self.colors.push(rgba);
        }
        self.indices
            .extend(triangles.into_iter().map(|index| base + index as u32));
    }

    /// Полилиния как цепочка квадов; каждый конец сегмента продлён на
    /// полширины, чтобы стыки перекрывались (как у старых дорог-спрайтов).
    pub fn push_polyline(&mut self, points: &[Vec2], width: f32, color: LinearRgba) {
        for segment in points.windows(2) {
            let Some(direction) = (segment[1] - segment[0]).try_normalize() else {
                continue;
            };
            let extension = direction * width / 2.0;
            let normal = direction.perp() * width / 2.0;
            let from = segment[0] - extension;
            let to = segment[1] + extension;
            self.push_quad(
                [from + normal, from - normal, to - normal, to + normal],
                color,
            );
        }
    }

    /// Прямоугольник по AABB (для тайловых оверлеев).
    pub fn push_rect(&mut self, min: Vec2, max: Vec2, color: LinearRgba) {
        self.push_quad(
            [
                Vec2::new(min.x, min.y),
                Vec2::new(max.x, min.y),
                Vec2::new(max.x, max.y),
                Vec2::new(min.x, max.y),
            ],
            color,
        );
    }

    fn push_quad(&mut self, corners: [Vec2; 4], color: LinearRgba) {
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        for corner in corners {
            self.positions.push([corner.x, corner.y, 0.0]);
            self.colors.push(rgba);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn build(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_with_hole_triangulates() {
        let mut builder = MeshBuilder::default();
        builder.push_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(0.0, 10.0),
            ],
            &[vec![
                Vec2::new(4.0, 4.0),
                Vec2::new(6.0, 4.0),
                Vec2::new(6.0, 6.0),
                Vec2::new(4.0, 6.0),
            ]],
            LinearRgba::WHITE,
        );
        assert!(!builder.is_empty());
        assert_eq!(builder.skipped_polygons(), 0);
        assert_eq!(builder.positions.len(), 8);
        // квадрат с дыркой — 8 треугольников
        assert_eq!(builder.indices.len(), 24);
    }

    #[test]
    fn degenerate_polygon_is_skipped() {
        let mut builder = MeshBuilder::default();
        builder.push_polygon(&[Vec2::ZERO, Vec2::new(1.0, 1.0)], &[], LinearRgba::WHITE);
        assert!(builder.is_empty());
        assert_eq!(builder.skipped_polygons(), 1);
    }

    #[test]
    fn polyline_makes_quad_per_segment() {
        let mut builder = MeshBuilder::default();
        builder.push_polyline(
            &[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
            2.0,
            LinearRgba::WHITE,
        );
        assert_eq!(builder.positions.len(), 8);
        assert_eq!(builder.indices.len(), 12);
    }
}
