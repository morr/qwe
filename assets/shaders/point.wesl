// Точечный материал: атлас силуэтов + вершинный цвет, но квад разворачивается
// в вершинном шейдере из атрибутов «центр + полуразмер + пол», а не из углов.
// Его едят запечённые трупы (`silhouette::bake` — трупные чанки) и дальний
// слой живых людей (Фаза 5 ×10).
//
// Зачем разворот на ГПУ: запечённый чанк не перестраивается на смену зума —
// пол размера (`HUMAN_MIN_PX`) применяется здесь, по юниформу зума, той же
// формулой, что `Silhouette::drawn_size` у спрайтов. Перестройка чанка нужна
// только когда прибывают новые трупы, а камера может двигаться свободно.
//
// Атлас — тот же растр RGBA8 с мипами и линейной фильтрацией
// (`silhouette/mod.rs`), что у свежих спрайтов: тексель умножается на цвет
// вершины (rgb × rgb, alpha × alpha), кромка сглажена, прозрачные тексели
// несут цвет кромки.
import bevy_sprite_render::mesh2d::{functions as mesh_functions, view_bindings::view};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

// Зеркало `bake::PointUniform` — порядок полей обязан совпадать.
struct PointUniform {
    // метров на логический пиксель (зум камеры); переписывает
    // `bake::sync_point_uniform` вслед за `Viewport`
    mpp: f32,
    // резерв раскладки: uv считается при сборке меша, поле держит структуру
    // кратной 16 байтам
    inv_cells: f32,
    pad0: f32,
    pad1: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: PointUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var atlas_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atlas_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    // угол квада, ±1 по обеим осям, уже повёрнутый на угол квадада при сборке
    // меша; зеркало глифа живёт в uv (`bake::chunk_mesh`) и намотку не трогает
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // центр квада в метрах, полуразмер (квад — квадрат), пол в логических px
    @location(2) quad: vec4<f32>,
    // uv вершины в атласе целиком: кромка ячейки, зеркало и вертикаль
    // посчитаны при сборке меша (`bake::chunk_mesh`) — угол квадада уже
    // запечён в position, и выводить uv из него было бы ошибкой
    @location(3) cell_uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let corner = vertex.position.xy;
    // `Silhouette::drawn_size`: тело, растянутое до пола, если тело на экране
    // меньше пола. Квад квадратный, меньшая сторона — 2·полуразмер.
    let drawn_half = vertex.quad.z
        * max(params.mpp * vertex.quad.w / (2.0 * vertex.quad.z), 1.0);
    // центр уже в метрах мира; трансформ чанка несёт только z-полку
    let local = vec3<f32>(vertex.quad.xy + corner * drawn_half, 0.0);
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world_position = mesh_functions::mesh2d_position_local_to_world(
        world_from_local,
        vec4<f32>(local, 1.0),
    );
    var out: VertexOutput;
    out.position = mesh_functions::mesh2d_position_world_to_clip(world_position);
    out.color = vertex.color;
    out.uv = vertex.cell_uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas_texture, atlas_sampler, in.uv);
    var output_color = vec4<f32>(in.color.rgb * texel.rgb, in.color.a * texel.a);
@if(TONEMAP_IN_SHADER)
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
@if(SRGB_OUTPUT)
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
@if(OKLAB_OUTPUT)
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
    return output_color;
}
