// Портал: вихрь, посчитанный в пикселе, — ни текстур, ни кадров спрайтшита.
// Квад с UV 0…1 → полярные координаты; рукава — косинус закрученного угла
// (лог-спираль: закрутка растёт к центру), турбулентность и рваный край — fbm
// из шума значений. Цвета и параметры приходят из `PortalMaterial`
// (`src/portal.rs`); яркость рукавов задаётся выше 1.0 намеренно — это HDR под
// bloom камеры (`src/camera.rs`), без него они просто выбелятся.

#import bevy_sprite::{mesh2d_vertex_output::VertexOutput, mesh2d_view_bindings::globals}

struct PortalUniform {
    // цвет ядра: почти чёрный
    core: vec4<f32>,
    // цвет рукавов и ободка, HDR
    rim: vec4<f32>,
    // x: скорость вращения, рад/с; y: число рукавов; z: закрутка; w: зерно шума
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> portal: PortalUniform;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        value += amplitude * value_noise(q);
        q = q * 2.03 + vec2<f32>(17.0, 9.0);
        amplitude *= 0.5;
    }
    return value;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let r = length(uv);
    let time = globals.time;
    let spin = portal.params.x;
    let arms = portal.params.y;
    let twist = portal.params.z;
    let grain = portal.params.w;

    // закрученный угол: чем ближе к центру, тем сильнее — лог-спираль
    let angle = atan2(uv.y, uv.x);
    let swirl = angle + twist * log(max(r, 0.03)) - time * spin;
    let arm = 0.5 + 0.5 * cos(arms * swirl);
    // турбулентность: плывёт к центру и вращается вместе с рукавами
    let n = fbm(vec2<f32>(r * grain - time * 0.6, swirl * 1.5));

    // рваный край с мягким спадом
    let rim_edge = 1.0 - 0.10 * n;
    let coverage = smoothstep(rim_edge, rim_edge - 0.12, r);
    // тёмное ядро, тоже рваное
    let core = smoothstep(0.42, 0.0, r + 0.08 * n);
    // рукава — между ядром и краем
    let arm_band = smoothstep(0.15, 0.55, r) * smoothstep(1.0, 0.65, r);
    let glow = arm_band * pow(arm, 3.0) * (0.6 + 0.8 * n);
    // тонкий яркий ободок у самого края
    let ring = smoothstep(0.10, 0.0, abs(r - rim_edge + 0.06));

    var color = mix(portal.rim.rgb * 0.35, portal.core.rgb, core);
    color += portal.rim.rgb * glow;
    color += portal.rim.rgb * ring * 0.8;
    let alpha = coverage * (0.85 + 0.15 * n);
    return vec4<f32>(color, alpha);
}
