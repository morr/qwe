//! Сборка слитых 2D-мешей слоёв карты: тысячи полигонов OSM в один
//! `Mesh2d` с вершинными цветами (стоковый `ColorMaterial` их умножает).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

/// Максимальное удлинение стыка ленты относительно полуширины. Контур кроны
/// полон почти встречных рёбер (впадины между фестонами), и там miter уходит
/// в длинный шип — при 1.5 стык вырождается в срез, шипов не видно.
const MITER_LIMIT: f32 = 1.5;

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

    /// Сколько вершин уже накоплено — тестам, чтобы сравнивать объём
    /// геометрии, не разбирая готовый меш.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Накопленные вершины — только для тестов, которым надо проверить, куда
    /// именно легла геометрия (высота проёма арки, например).
    #[cfg(test)]
    pub fn positions_for_test(&self) -> &[[f32; 3]] {
        &self.positions
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

    /// Уже собранная геометрия, приложенная со сдвигом и масштабом. Силуэт
    /// тени кроны один на вариант и повторяется под тысячами деревьев —
    /// триангулировать его каждый раз заново незачем.
    pub fn push_template(&mut self, template: &MeshBuilder, offset: Vec2, scale: f32) {
        let base = self.positions.len() as u32;
        self.positions
            .extend(template.positions.iter().map(|point| {
                [
                    point[0] * scale + offset.x,
                    point[1] * scale + offset.y,
                    point[2],
                ]
            }));
        self.colors.extend_from_slice(&template.colors);
        self.indices
            .extend(template.indices.iter().map(|index| base + index));
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

    /// Лента постоянной ширины вдоль ломаной: стыки сведены по биссектрисе
    /// (miter с ограничением `MITER_LIMIT`), торцы **не** продлеваются.
    /// Для тонких контуров `push_polyline` не годится — там каждый сегмент
    /// продлён на полширины, и на ломаной с сегментами короче ширины штриха
    /// (контур кроны) продления соседних квадов торчат наружу шипами.
    /// Точки ближе `width / 4` к предыдущей отбрасываются: на такой дистанции
    /// они не видны, но вырождают нормаль стыка.
    pub fn push_stroke(&mut self, points: &[Vec2], closed: bool, width: f32, color: LinearRgba) {
        let merge_distance_sq = (width / 4.0).powi(2);
        let mut path: Vec<Vec2> = Vec::with_capacity(points.len());
        for &point in points {
            if path
                .last()
                .is_none_or(|last| last.distance_squared(point) > merge_distance_sq)
            {
                path.push(point);
            }
        }
        if closed {
            while path.len() > 1
                && path[0].distance_squared(path[path.len() - 1]) <= merge_distance_sq
            {
                path.pop();
            }
        }
        if path.len() < 2 {
            return;
        }

        let half_width = width / 2.0;
        let count = path.len();
        let offsets: Vec<Vec2> = (0..count)
            .map(|index| {
                let incoming = (index > 0 || closed).then(|| {
                    let previous = path[(index + count - 1) % count];
                    (path[index] - previous).normalize_or(Vec2::X).perp()
                });
                let outgoing = (index + 1 < count || closed).then(|| {
                    let next = path[(index + 1) % count];
                    (next - path[index]).normalize_or(Vec2::X).perp()
                });
                match (incoming, outgoing) {
                    (Some(before), Some(after)) => {
                        let bisector = (before + after).normalize_or(before);
                        // на острых стыках длина miter уходит в бесконечность — режем
                        let cosine = bisector.dot(before).max(1.0 / MITER_LIMIT);
                        bisector * (half_width / cosine)
                    }
                    (Some(normal), None) | (None, Some(normal)) => normal * half_width,
                    (None, None) => Vec2::ZERO,
                }
            })
            .collect();

        let segments = if closed { count } else { count - 1 };
        for index in 0..segments {
            let next = (index + 1) % count;
            self.push_quad(
                [
                    path[index] + offsets[index],
                    path[index] - offsets[index],
                    path[next] - offsets[next],
                    path[next] + offsets[next],
                ],
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

    pub(crate) fn push_quad(&mut self, corners: [Vec2; 4], color: LinearRgba) {
        self.push_quad_gradient(corners, [color; 4]);
    }

    /// Квад с цветом на каждую вершину — для вертикального градиента стен
    /// экструдированных зданий.
    pub(crate) fn push_quad_gradient(&mut self, corners: [Vec2; 4], colors: [LinearRgba; 4]) {
        let base = self.positions.len() as u32;
        for (corner, color) in corners.into_iter().zip(colors) {
            self.positions.push([corner.x, corner.y, 0.0]);
            self.colors.push(color.to_f32_array());
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
mod tests;
