// Полог кроны сверху. Геометрию делает `map/trees/crown.rs` (алгоритм
// watabou), здесь — только освещение и листва.
//
// Крона на снимке — не силуэт с обводкой, а **шар листвы**: одна сторона
// освещена, противоположная в собственной тени, и весь он покрыт рябью
// отдельных ветвей. Мешу об этом знать нечего: он единичного радиуса и
// повторён тысячами деревьев, поэтому и то и другое считается здесь —
// освещение по **локальной** координате (её хватает: меш нормирован), рябь по
// **мировой** (иначе соседние деревья одного варианта были бы близнецами).

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::view,
}
#import "shaders/noise.wgsl"::{fbm3, visible}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
#ifdef SRGB_OUTPUT
#import bevy_render::color_operations::linear_to_srgb
#endif
#ifdef OKLAB_OUTPUT
#import bevy_render::color_operations::linear_rgb_to_oklab
#endif

// Зеркало `trees::CrownParamsUniform` — порядок полей обязан совпадать.
struct CrownUniform {
    light: vec2<f32>,
    // множитель яркости этого дерева (`TreeStyle::tint_factors`)
    tint: f32,
    intensity: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CrownUniform;

// Насколько освещённая сторона кроны светлее средней, а теневая темнее.
// Шар, а не диск: к краю тени добавляется падение по радиусу.
const LIT: f32 = 0.20;
const RIM: f32 = 0.10;
// Рябь листвы: длина волны в метрах и её сила.
const LEAF_SCALE: f32 = 0.9;
const LEAF_AMP: f32 = 0.16;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec2<f32>,
    @location(1) color: vec4<f32>,
    // координата внутри кроны: меш единичного радиуса, так что это сразу
    // направление от ствола и расстояние до края
    @location(2) local: vec2<f32>,
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
    out.local = vertex.position.xy;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb * params.tint;

    if params.intensity > 0.0 {
        let p = in.world_position;
        let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
        // сторона к солнцу светлее, противоположная темнее; у края добавляется
        // падение по радиусу — крона всё-таки шар, а не блин
        let lit = dot(in.local, params.light);
        let reach = length(in.local);
        var shade = LIT * lit - RIM * reach * reach;
        // рябь отдельных ветвей — по мировой точке, поэтому два дерева одного
        // варианта рядом не выглядят копиями
        shade += LEAF_AMP * fbm3(p + vec2<f32>(31.0, 17.0), LEAF_SCALE, px);
        // тень внутри полога холоднее, свет теплее — как и на кровле
        let tint = vec3<f32>(0.30, 0.10, -0.25) * shade;
        rgb = rgb * (1.0 + params.intensity * (vec3<f32>(shade) + tint));
    }

    var output_color = vec4<f32>(rgb, in.color.a);
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
