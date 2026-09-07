// Процедурная фактура поверхностей карты — земля, зелень, песок, вода,
// асфальт. Материал и смысл параметров — `src/map/surface.rs`.
//
// Базовый цвет приходит вершинным, как у `ColorMaterial`; шейдер умножает его
// на шум по **мировым** координатам, поэтому две перекрывающиеся ленты одного
// слоя (стык дорог в узле) красятся в один и тот же пиксель одинаково и стык
// остаётся невидимым — то самое свойство, ради которого слой дорог плоский.
//
// Все октавы шума гасятся по размеру пикселя (`fwidth` мировой координаты):
// волна короче пары пикселей не сэмплируется, а исчезает, иначе на отдалении
// зерно превращается в муар и мерцает при движении камеры.

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::{view, globals},
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
#ifdef SRGB_OUTPUT
#import bevy_render::color_operations::linear_to_srgb
#endif
#ifdef OKLAB_OUTPUT
#import bevy_render::color_operations::linear_rgb_to_oklab
#endif

// Зеркало `surface::SurfaceParams` — порядок полей обязан совпадать.
struct SurfaceParams {
    tint: vec4<f32>,
    marking_color: vec4<f32>,
    mottle_amp: f32,
    mottle_scale: f32,
    grain_amp: f32,
    grain_scale: f32,
    speckle_amp: f32,
    speckle_scale: f32,
    speckle_threshold: f32,
    drift: f32,
    marking_width: f32,
    marking_dash: f32,
    marking_gap: f32,
    intensity: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SurfaceParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_RIBBON`: поперёк ленты (м), до разрыва разметки (м,
    // внутри разрыва отрицательно), полуширина (м), код разметки
    // (полосы · 2 + односторонняя; 0 — без разметки)
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

// Хеш вещественной пары в [0, 1) (Dave Hoskins, hash12).
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Value noise, центрированный: [-0.5, 0.5]. Центрирован намеренно — погашенная
// октава тогда вносит ровно ноль, а не сдвигает среднюю яркость с зумом.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y) - 0.5;
}

// Видимость волны длиной `wavelength` при `px` метрах на пиксель: короче
// полутора пикселей — ноль, длиннее четырёх — единица.
fn visible(wavelength: f32, px: f32) -> f32 {
    return smoothstep(1.5, 4.0, wavelength / px);
}

// Четыре октавы от `scale` вниз (до `scale / 8`), ~[-1, 1] — облачность:
// от пятен в десятки метров до пятен в несколько. Нормировка — по полным
// амплитудам, а не по видимым: погашенная октава уменьшает контраст (на
// отдалении поверхность становится ровнее), но не усиливает оставшиеся.
fn fbm4(p: vec2<f32>, scale: f32, px: f32) -> f32 {
    let n0 = value_noise(p / scale) * visible(scale, px);
    let n1 = value_noise(p / (scale * 0.5)) * visible(scale * 0.5, px);
    let n2 = value_noise(p / (scale * 0.25)) * visible(scale * 0.25, px);
    let n3 = value_noise(p / (scale * 0.125)) * visible(scale * 0.125, px);
    return (n0 + 0.5 * n1 + 0.25 * n2 + 0.125 * n3) / 1.875 * 2.0;
}

// Три октавы (до `scale / 4`) — зерно: рябь в метры и доли метра, видная
// только вблизи.
fn fbm3(p: vec2<f32>, scale: f32, px: f32) -> f32 {
    let n0 = value_noise(p / scale) * visible(scale, px);
    let n1 = value_noise(p / (scale * 0.5)) * visible(scale * 0.5, px);
    let n2 = value_noise(p / (scale * 0.25)) * visible(scale * 0.25, px);
    return (n0 + 0.5 * n1 + 0.25 * n2) / 1.75 * 2.0;
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

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position;
    // метров на пиксель; камера без поворота, так что обе производные равны
    let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
    let k = params.intensity;

    var rgb = in.color.rgb;

    // крупная «облачность»: яркость плюс тёплый/холодный сдвиг тона. Дрейф —
    // только у воды (у прочих `drift` ноль): рябь медленно плывёт по глади
    let drift = globals.time * params.drift;
    let cloud = fbm4(p + vec2<f32>(3.1 + drift, 7.7 + drift * 0.6), params.mottle_scale, px);
    rgb = rgb * (1.0 + k * cloud * (vec3<f32>(params.mottle_amp) + params.tint.rgb));

    // мелкое зерно
    let grain = fbm3(p + vec2<f32>(11.3, 5.9), params.grain_scale, px);
    rgb = rgb * (1.0 + k * params.grain_amp * grain);

    // крапинки — редкие тёмные пятна там, где отдельный шум выше порога:
    // кочки травы, подлесок. Порог задаёт редкость, шаг — размер
    if params.speckle_amp > 0.0 {
        let field = value_noise((p + vec2<f32>(23.7, 41.1)) / params.speckle_scale) + 0.5;
        let dots = smoothstep(params.speckle_threshold, params.speckle_threshold + 0.12, field)
            * visible(params.speckle_scale, px);
        rgb = rgb * (1.0 - k * params.speckle_amp * dots);
    }

    // разметка проезжей части по локальным координатам ленты: линия на
    // каждой границе полос — штриховая, а осевая многополосной двусторонней
    // сплошная. Линия не у́же ~1.3 px (тоньше — мерцает при сдвиге камеры),
    // со сглаженным краем; гаснет в разрыве у перекрёстка (`to_break` < 0) и
    // когда полоса на экране у́же десятка пикселей
    let mode = u32(round(max(in.ribbon.w, 0.0)));
    if params.marking_width > 0.0 && mode >= 4u {
        let lanes = f32(mode >> 1u);
        let oneway = (mode & 1u) == 1u;
        let across = in.ribbon.x;
        let to_break = in.ribbon.y;
        let half_width = in.ribbon.z;
        let lane_width = 2.0 * half_width / lanes;
        // ближайшая граница полос, считая от края: 0 и `lanes` — края ленты
        let boundary = round((across + half_width) / lane_width);
        let inside = boundary >= 1.0 && boundary <= lanes - 1.0;
        let to_boundary = abs(across + half_width - boundary * lane_width);
        let line = max(params.marking_width, 1.3 * px);
        let edge = 0.7 * px;
        let on_line = 1.0 - smoothstep(line * 0.5 - edge, line * 0.5 + edge, to_boundary);
        // осевая — граница ровно посередине двусторонней ленты
        let axis = !oneway && boundary * 2.0 == lanes;
        let solid = axis && lanes >= 4.0;
        // штрихи считаются от края разрыва, и первым идёт пропуск: линия не
        // упирается в перекрёсток штрихом
        let dash = dash_distance(to_break - params.marking_gap, params.marking_dash, params.marking_gap);
        let on_dash = select(1.0 - smoothstep(-edge, edge, dash), 1.0, solid);
        let gap_fade = smoothstep(0.0, 1.0, to_break);
        let zoom_fade = smoothstep(6.0, 12.0, lane_width / px);
        let mask = on_line * on_dash * gap_fade * zoom_fade * params.marking_color.a * f32(inside);
        rgb = mix(rgb, params.marking_color.rgb, mask);
    }

    // слой непрозрачный: вода, дороги и зелень — сплошные заливки
    var output_color = vec4<f32>(rgb, 1.0);
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
