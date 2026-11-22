//! Второй impl-блок сборщика: клип собранной геометрии окном
//! ([`MeshBuilder::clip_to_rect`]), нарезка слоя на куски по клеткам карты
//! ([`MeshBuilder::into_chunks`]) и склейки кусков
//! ([`MeshBuilder::concat`], [`MeshBuilder::append`]) — всё, что работает
//! с геометрией после того, как она собрана.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::{MeshBuilder, NO_RIBBON, NO_ROOF};

impl MeshBuilder {
    /// Обрезать уже собранную геометрию прямоугольником `min..max`: каждый
    /// треугольник режется по четырём сторонам, вершины на линии реза получают
    /// цвет и координаты фактуры **линейной интерполяцией** — той же, какой их
    /// по треугольнику ведёт GPU, так что внутри окна картинка не меняется ни
    /// на пиксель.
    ///
    /// Нужна витрине пересечений (`examples/demos/roads`): та ставит окна
    /// разных мест города встык, и всё, что слой рисует за своим окном — хвост
    /// дороги, целый дом на краю вырезки, квад земли во всю карту, — иначе
    /// ложится в окно соседа. Игре она не нужна: у неё карта одна.
    pub fn clip_to_rect(&mut self, min: Vec2, max: Vec2) {
        /// Вершина со всем, что на ней едет: позиция (с z), цвет, две
        /// необязательные четвёрки фактуры и необязательная координата кроны.
        type Vertex = ([f32; 3], [f32; 4], [f32; 4], [f32; 4], [f32; 2]);
        fn mix<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        }

        let vertex = |index: u32| -> Vertex {
            let i = index as usize;
            (
                self.positions[i],
                self.colors[i],
                self.ribbon.as_ref().map_or([0.0; 4], |ribbon| ribbon[i]),
                self.roof.as_ref().map_or([0.0; 4], |roof| roof[i]),
                self.crown.as_ref().map_or([0.0; 2], |crown| crown[i]),
            )
        };
        // сторона окна: ось, граница и с какой её стороны — «внутри»
        let sides = [
            (0, min.x, 1.0),
            (0, max.x, -1.0),
            (1, min.y, 1.0),
            (1, max.y, -1.0),
        ];

        let mut kept: Vec<Vertex> = Vec::new();
        let mut indices = Vec::new();
        for triangle in self.indices.chunks_exact(3) {
            let mut polygon: Vec<Vertex> = triangle.iter().map(|index| vertex(*index)).collect();
            for (axis, bound, sign) in sides {
                let inside = |v: &Vertex| (v.0[axis] - bound) * sign >= 0.0;
                let mut clipped = Vec::with_capacity(polygon.len() + 1);
                for (i, a) in polygon.iter().enumerate() {
                    let b = &polygon[(i + 1) % polygon.len()];
                    if inside(a) {
                        clipped.push(*a);
                    }
                    if inside(a) != inside(b) {
                        let t = (bound - a.0[axis]) / (b.0[axis] - a.0[axis]);
                        clipped.push((
                            mix(a.0, b.0, t),
                            mix(a.1, b.1, t),
                            mix(a.2, b.2, t),
                            mix(a.3, b.3, t),
                            mix(a.4, b.4, t),
                        ));
                    }
                }
                polygon = clipped;
                if polygon.is_empty() {
                    break;
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            // треугольник, обрезанный выпуклым окном, выпукл — веер от первой
            let base = kept.len() as u32;
            kept.extend(&polygon);
            for i in 1..polygon.len() as u32 - 1 {
                indices.extend([base, base + i, base + i + 1]);
            }
        }

        self.positions = kept.iter().map(|vertex| vertex.0).collect();
        self.colors = kept.iter().map(|vertex| vertex.1).collect();
        if let Some(ribbon) = &mut self.ribbon {
            *ribbon = kept.iter().map(|vertex| vertex.2).collect();
        }
        if let Some(roof) = &mut self.roof {
            *roof = kept.iter().map(|vertex| vertex.3).collect();
        }
        if let Some(crown) = &mut self.crown {
            *crown = kept.iter().map(|vertex| vertex.4).collect();
        }
        self.indices = indices;
    }

    /// Разрезать собранную геометрию по клеткам `cell` метров: каждый
    /// треугольник **целиком** уходит в клетку своего центроида. Обратная
    /// операция тривиальна ([`Self::merge`]) и объединение кусков равно
    /// исходной геометрии байт в байт по накрытию: ни швов, ни двойного
    /// рисования, порядок треугольников внутри куска — исходный.
    ///
    /// Зачем: слой-меш на всю карту несёт Aabb всей карты, и frustum culling
    /// его не отбрасывает никогда — каждый треугольник уходит в GPU на любом
    /// зуме. Куски по клеткам возвращают отсечению смысл: кадр ближнего зума
    /// видит одну-две клетки, и весь остальной слой не рисуется вовсе.
    ///
    /// Вершина, общая треугольникам из разных клеток, копируется в каждый —
    /// суммарный счёт вершин растёт на долю приграничных, а атрибуты
    /// (координаты ленты, рамки кровли, координаты кроны) переезжают с
    /// вершиной как записаны. Порядок кусков — по клеткам (y, x), как у крон
    /// (`trees::sorted_chunks`): детерминированный, от хэшера не зависит.
    pub fn into_chunks(self, cell: f32) -> Vec<MeshBuilder> {
        let Self {
            positions,
            colors,
            indices,
            ribbon,
            roof,
            crown,
            ..
        } = self;
        /// Кусок: сборщик с тем же набором атрибутов, что у источника, плюс
        /// карта «старый индекс вершины → индекс в куске».
        struct Chunk {
            builder: MeshBuilder,
            remap: HashMap<u32, u32>,
        }
        let mut chunks: HashMap<IVec2, Chunk> = HashMap::new();
        for triangle in indices.chunks_exact(3) {
            let cell_of = |index: u32| {
                let [x, y, _] = positions[index as usize];
                Vec2::new(x, y)
            };
            // центроид по вершинам — треугольник может висеть через границу
            let (a, b, c) = (
                cell_of(triangle[0]),
                cell_of(triangle[1]),
                cell_of(triangle[2]),
            );
            let centroid = (a + b + c) / 3.0;
            let key = IVec2::new(
                (centroid.x / cell).floor() as i32,
                (centroid.y / cell).floor() as i32,
            );
            let Chunk { builder, remap } = chunks.entry(key).or_insert_with(|| Chunk {
                builder: MeshBuilder {
                    ribbon: ribbon.is_some().then(Vec::new),
                    roof: roof.is_some().then(Vec::new),
                    crown: crown.is_some().then(Vec::new),
                    ..MeshBuilder::default()
                },
                remap: HashMap::new(),
            });
            for &old in triangle {
                let new = match remap.get(&old) {
                    Some(&new) => new,
                    None => {
                        let index = old as usize;
                        let new = builder.positions.len() as u32;
                        builder.positions.push(positions[index]);
                        builder.colors.push(colors[index]);
                        if let Some(coords) = &mut builder.ribbon {
                            coords.push(ribbon.as_ref().map_or(NO_RIBBON, |r| r[index]));
                        }
                        if let Some(frames) = &mut builder.roof {
                            frames.push(roof.as_ref().map_or(NO_ROOF, |r| r[index]));
                        }
                        if let Some(local) = &mut builder.crown {
                            local.push(crown.as_ref().map_or([0.0; 2], |c| c[index]));
                        }
                        remap.insert(old, new);
                        new
                    }
                };
                builder.indices.push(new);
            }
        }
        sorted_by_cell(chunks)
            .into_iter()
            .map(|Chunk { builder, .. }| builder)
            .collect()
    }

    /// Склеить куски [`Self::into_chunks`] обратно в один сборщик — индексы
    /// каждого куска сдвигаются на вершины кусков перед ним, атрибуты
    /// переезжают с вершинами. Обратная операция нужна только тестам: слой
    /// с фактурой в игре живёт кусками, и склеивать его некому
    /// (`MeshBuilder::concat` склеивает лишь куски без атрибутов).
    #[cfg(test)]
    pub(crate) fn merge(parts: Vec<MeshBuilder>) -> MeshBuilder {
        let mut merged = MeshBuilder::default();
        let Some(first) = parts.first() else {
            return merged;
        };
        // куски одного сборщика несут один и тот же набор атрибутов
        merged.ribbon = first.ribbon.is_some().then(Vec::new);
        merged.roof = first.roof.is_some().then(Vec::new);
        merged.crown = first.crown.is_some().then(Vec::new);
        for part in parts {
            let base = merged.positions.len() as u32;
            merged.positions.extend(part.positions);
            merged.colors.extend(part.colors);
            merged
                .indices
                .extend(part.indices.iter().map(|index| base + index));
            if let Some(ribbon) = &mut merged.ribbon {
                ribbon.extend(part.ribbon.expect("attribute set must match"));
            }
            if let Some(roof) = &mut merged.roof {
                roof.extend(part.roof.expect("attribute set must match"));
            }
            if let Some(crown) = &mut merged.crown {
                crown.extend(part.crown.expect("attribute set must match"));
            }
        }
        merged
    }

    /// Склеить куски в один сборщик — ровно в том порядке, в каком их
    /// геометрия легла бы в один сборщик подряд: индексы каждого куска
    /// сдвигаются на вершины кусков перед ним, так что меш выходит тем же байт
    /// в байт. Так собирается слой, чьи куски строятся по потокам
    /// (`cars::mesh_bodies`).
    ///
    /// Копирование идёт **тоже по потокам** — по `groups` подряд идущих
    /// кусков на поток, в заранее выделенные буферы: склейка в один поток
    /// стоила на слое машин Калуги (полтора миллиона вершин, полсотни
    /// мегабайт) столько же, сколько сама сборка кусков, — это копирование и
    /// первое касание свежей памяти, а не арифметика. Буферы выделяются нулями
    /// (`vec![0; n]` берёт у системы уже обнулённые страницы), так что первое
    /// касание достаётся потокам.
    ///
    /// Только куски без фактуры (ни ленты, ни кровли, ни координат кроны):
    /// склеивать их пока нечему, а атрибут, которого нет у одного из кусков,
    /// лёг бы не на каждую вершину.
    pub fn concat(parts: &[MeshBuilder], groups: usize) -> MeshBuilder {
        assert!(
            parts
                .iter()
                .all(|part| part.ribbon.is_none() && part.roof.is_none() && part.crown.is_none()),
            "concat: куски с фактурой не склеиваются"
        );
        let vertices = parts.iter().map(|part| part.positions.len()).sum();
        let index_count = parts.iter().map(|part| part.indices.len()).sum();
        let mut positions = vec![[0.0; 3]; vertices];
        let mut colors = vec![[0.0; 4]; vertices];
        let mut indices = vec![0; index_count];

        /// Куда ляжет один кусок: его доли трёх буферов и сдвиг индексов.
        struct Slot<'a> {
            part: &'a MeshBuilder,
            positions: &'a mut [[f32; 3]],
            colors: &'a mut [[f32; 4]],
            indices: &'a mut [u32],
            offset: u32,
        }
        let mut slots = Vec::with_capacity(parts.len());
        let (mut positions_rest, mut colors_rest, mut indices_rest) = (
            positions.as_mut_slice(),
            colors.as_mut_slice(),
            indices.as_mut_slice(),
        );
        let mut offset = 0u32;
        for part in parts {
            let count = part.positions.len();
            let (to_positions, rest) = positions_rest.split_at_mut(count);
            positions_rest = rest;
            let (to_colors, rest) = colors_rest.split_at_mut(count);
            colors_rest = rest;
            let (to_indices, rest) = indices_rest.split_at_mut(part.indices.len());
            indices_rest = rest;
            slots.push(Slot {
                part,
                positions: to_positions,
                colors: to_colors,
                indices: to_indices,
                offset,
            });
            offset += count as u32;
        }
        let copy = |slots: &mut [Slot]| {
            for slot in slots {
                slot.positions.copy_from_slice(&slot.part.positions);
                slot.colors.copy_from_slice(&slot.part.colors);
                for (to, from) in slot.indices.iter_mut().zip(&slot.part.indices) {
                    *to = from + slot.offset;
                }
            }
        };
        let per_group = slots.len().div_ceil(groups.max(1)).max(1);
        std::thread::scope(|scope| {
            let mut chunks = slots.chunks_mut(per_group);
            // первая группа — на вызывающем потоке: ему всё равно ждать
            let first = chunks.next();
            for chunk in chunks {
                scope.spawn(move || copy(chunk));
            }
            if let Some(chunk) = first {
                copy(chunk);
            }
        });
        MeshBuilder {
            positions,
            colors,
            indices,
            skipped_polygons: parts.iter().map(|part| part.skipped_polygons).sum(),
            ..MeshBuilder::default()
        }
    }

    /// Дописать собранное `other` в конец — порядок треугольников `other`
    /// сохраняется. Фактуры (`ribbon`/`roof`/`crown`) сливаются, когда есть у
    /// обоих, и заводятся у пустого получателя; свести `None` к `Some` у
    /// непустого нельзя — индексы поехали бы мимо атрибута, это паника.
    /// Раскладки будущей геометрии (`lanes`, `lane_profile`, `frame`) —
    /// состояние пишущего: у дописанного куска их нет.
    pub fn append(&mut self, other: &MeshBuilder) {
        let was_empty = self.positions.is_empty();
        let offset = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.colors.extend_from_slice(&other.colors);
        self.indices
            .extend(other.indices.iter().map(|&index| index + offset));
        self.skipped_polygons += other.skipped_polygons;
        merge_attr(&mut self.ribbon, other.ribbon.as_deref(), was_empty);
        merge_attr(&mut self.roof, other.roof.as_deref(), was_empty);
        merge_attr(&mut self.crown, other.crown.as_deref(), was_empty);
    }

    /// Зарезервировать место под `vertices` вершин и `indices` индексов —
    /// когда объём геометрии известен заранее хотя бы оценкой.
    pub fn reserve(&mut self, vertices: usize, indices: usize) {
        self.positions.reserve(vertices);
        self.colors.reserve(vertices);
        self.indices.reserve(indices);
        if let Some(ribbon) = &mut self.ribbon {
            ribbon.reserve(vertices);
        }
        if let Some(roof) = &mut self.roof {
            roof.reserve(vertices);
        }
        if let Some(crown) = &mut self.crown {
            crown.reserve(vertices);
        }
    }

    /// Сколько индексов уже накоплено — отметка, от которой
    /// [`Self::area_since`] считает площадь.
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    /// Сумма площадей треугольников, положенных после отметки `first_index`
    /// ([`Self::index_count`]). Перекрытия считаются дважды — это площадь
    /// нарисованного, а не покрытого.
    pub fn area_since(&self, first_index: usize) -> f32 {
        let point = |index: u32| {
            let [x, y, _] = self.positions[index as usize];
            Vec2::new(x, y)
        };
        self.indices[first_index..]
            .chunks_exact(3)
            .map(|triangle| {
                let (a, b, c) = (point(triangle[0]), point(triangle[1]), point(triangle[2]));
                (b - a).perp_dot(c - a).abs() / 2.0
            })
            .sum()
    }
}

/// Значения кусков по клеткам карты — в порядке клеток (снизу вверх, слева
/// направо), а не обхода словаря: так порядок кусков и их z не зависят ни от
/// хэшера, ни от порядка вставки. Этим порядком заканчиваются и
/// [`MeshBuilder::into_chunks`], и куски крон (`trees::sorted_chunks`).
pub(crate) fn sorted_by_cell<T>(mut chunks: HashMap<IVec2, T>) -> Vec<T> {
    let mut keys: Vec<IVec2> = chunks.keys().copied().collect();
    keys.sort_by_key(|key| (key.y, key.x));
    keys.into_iter()
        .map(|key| chunks.remove(&key).expect("key came from the map"))
        .collect()
}

/// Одна фактура [`MeshBuilder::append`]: слияние при совпадающих наборах,
/// заведение у пустого получателя, паника при расходящемся.
fn merge_attr<T: Clone>(dst: &mut Option<Vec<T>>, src: Option<&[T]>, dst_was_empty: bool) {
    match (&mut *dst, src) {
        (_, None) => {}
        (slot @ None, Some(values)) => {
            assert!(
                dst_was_empty,
                "append: у непустого получателя нет атрибута, у куска есть"
            );
            *slot = Some(values.to_vec());
        }
        (Some(slot), Some(values)) => slot.extend(values.iter().cloned()),
    }
}
