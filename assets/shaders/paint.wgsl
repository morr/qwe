// Слой краски улиц — линии полос. Материал и смысл параметров —
// `src/map/roads/paint.rs`.
//
// Геометрия — полоса шире линии вдоль каждой линии (`MeshBuilder::
// push_paint_strip`); саму линию рисует этот шейдер по координатам полосы:
// ширину с полом в пиксели, сглаженный край, штрихи по длине улицы, сплошной
// подход к узлу, двойную сплошную и гашение с зумом. Так линия не мерцает при
// сдвиге камеры, а на дальнем плане гаснет, а не рвётся на отдельные пиксели.

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::view,
}
#import "shaders/noise.wgsl"::visible

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
#ifdef SRGB_OUTPUT
#import bevy_render::color_operations::linear_to_srgb
#endif
#ifdef OKLAB_OUTPUT
#import bevy_render::color_operations::linear_rgb_to_oklab
#endif

// Зеркало `roads::paint::PaintParams` — порядок полей обязан совпадать.
struct PaintParams {
    paint: f32,
    width: f32,
    dash: f32,
    gap: f32,
    approach: f32,
    double_offset: f32,
    lane_zoom: f32,
    axis_zoom: f32,
    fade_from: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PaintParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_RIBBON` полосы краски: поперёк от линии (м), длина
    // улицы (м), до разрыва перекрёстка (м), вид линии (0 — линия полос,
    // 1 — осевая, 2 — двойная сплошная)
    @location(2) ribbon: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) ribbon: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world_position = mesh_functions::mesh2d_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = mesh_functions::mesh2d_position_world_to_clip(world_position);
    out.world_position = world_position.xy;
    out.color = vertex.color;
    out.ribbon = vertex.ribbon;
    return out;
}

// Знаковое расстояние до штриха: отрицательно внутри штриха длиной `dash`,
// начинающегося в нуле каждого периода `dash + gap`.
fn dash_distance(along: f32, dash: f32, gap: f32) -> f32 {
    let period = dash + gap;
    let phase = along - floor(along / period) * period;
    let here = abs(phase - dash * 0.5) - dash * 0.5;
    let next = abs(phase - period - dash * 0.5) - dash * 0.5;
    return min(here, next);
}

// Покрытие линии ширины `width` на расстоянии `d` от её оси: пол 1.3 px,
// край сглажен на ±0.7 px.
fn line_cover(d: f32, width: f32, px: f32) -> f32 {
    let line = max(width, 1.3 * px);
    let edge = 0.7 * px;
    return 1.0 - smoothstep(line * 0.5 - edge, line * 0.5 + edge, d);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position;
    let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
    let across = in.ribbon.x;
    let along = in.ribbon.y;
    let to_break = in.ribbon.z;
    let kind = u32(round(in.ribbon.w));

    var cover = 0.0;
    if kind == 2u {
        // двойная сплошная: две линии по сторонам оси; когда зазор меньше пары
        // пикселей, они сливаются в одну осевую той же доли краски
        let pair = max(
            line_cover(abs(across - params.double_offset), params.width, px),
            line_cover(abs(across + params.double_offset), params.width, px),
        );
        let single = line_cover(abs(across), params.width * 2.0, px);
        let gap = 2.0 * params.double_offset - params.width;
        cover = mix(single, pair, smoothstep(1.5, 3.0, gap / px));
    } else {
        cover = line_cover(abs(across), params.width, px);
        // штрихи — по длине улицы, у узла линия сплошная. Где период штриха
        // меньше пары пикселей, штрих гаснет в свою среднюю долю, а не рябит
        let edge = 0.7 * px;
        let dash = dash_distance(along, params.dash, params.gap);
        let dashed = 1.0 - smoothstep(-edge, edge, dash);
        let mean = params.dash / (params.dash + params.gap);
        let seen = mix(mean, dashed, visible(params.dash + params.gap, px));
        let solid = 1.0 - smoothstep(params.approach - 1.0, params.approach, to_break);
        cover = cover * mix(seen, 1.0, solid);
    }
    // в разрыве перекрёстка линии нет; у края разрыва она гаснет за метр
    cover = cover * smoothstep(0.0, 1.0, to_break);
    // с зумом: линии полос уходят раньше осевых; к порогу, где меш прячется
    // (`roads::paint::PaintLods`), линия уже прозрачна
    let zoom_max = select(params.axis_zoom, params.lane_zoom, kind == 0u);
    cover = cover * (1.0 - smoothstep(zoom_max * params.fade_from, zoom_max, px));

    let alpha = cover * in.color.a * params.paint;
    var output_color = vec4<f32>(in.color.rgb, alpha);
#ifdef TONEMAP_IN_SHADER
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
#endif
#ifdef SRGB_OUTPUT
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
#endif
#ifdef OKLAB_OUTPUT
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
#endif
    return output_color;
}
