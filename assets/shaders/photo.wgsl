// Фотографический проход: чем снят кадр, а не что в нём. Смысл величин и
// почему они такие — `src/photo.rs`.
//
// Проход идёт после bloom и до тонмаппинга, по HDR-цели, поэтому всё, что
// ярче единицы, обязано остаться ярче единицы: кривая контраста применяется
// только к части в [0, 1], а превышение возвращается поверх неё.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Зеркало `photo::PhotoSettings` — порядок полей обязан совпадать.
struct PhotoSettings {
    grain: f32,
    haze: f32,
    aberration: f32,
    sharpen: f32,
    contrast: f32,
    amount: f32,
}

@group(0) @binding(2) var<uniform> settings: PhotoSettings;

// Цвет дымки: подсвеченный небом воздух между камерой и городом — холодный и
// очень слабый. Поднимает тени, светов не трогает.
const HAZE_TINT: vec3<f32> = vec3<f32>(0.035, 0.045, 0.062);

// Хеш экранной координаты в [0, 1) — тот же hash12 Дэйва Хоскинса, что даёт
// зерно поверхностям. По **пикселю**, а не по мировой точке: зерно живёт в
// сенсоре, и карта под ним едет, а оно стоит.
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let k = settings.amount;
    if k <= 0.0 {
        return textureSample(screen_texture, texture_sampler, uv);
    }

    let size = vec2<f32>(textureDimensions(screen_texture));
    let texel = 1.0 / size;

    // хроматическая аберрация: расхождение каналов растёт как квадрат
    // расстояния от центра кадра — в середине объектив чист
    let from_center = uv - vec2<f32>(0.5);
    let radial = from_center * (k * settings.aberration * dot(from_center, from_center) * 4.0);
    var color = vec3<f32>(
        textureSample(screen_texture, texture_sampler, uv + radial).r,
        textureSample(screen_texture, texture_sampler, uv).g,
        textureSample(screen_texture, texture_sampler, uv - radial).b,
    );

    // нерезкое маскирование: четыре диагональных отсчёта в пиксель дают
    // дешёвое размытие, разница с ним — ореол пан-шарпена
    let blur = (
        textureSample(screen_texture, texture_sampler, uv + texel * vec2<f32>(1.0, 1.0)).rgb
        + textureSample(screen_texture, texture_sampler, uv + texel * vec2<f32>(-1.0, 1.0)).rgb
        + textureSample(screen_texture, texture_sampler, uv + texel * vec2<f32>(1.0, -1.0)).rgb
        + textureSample(screen_texture, texture_sampler, uv + texel * vec2<f32>(-1.0, -1.0)).rgb
    ) * 0.25;
    color = color + k * settings.sharpen * (color - blur);

    // дымка — только в тени: чем темнее пиксель, тем больше воздуха между ним
    // и камерой видно
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = color + HAZE_TINT * (k * settings.haze * clamp(1.0 - luminance, 0.0, 1.0));

    // S-образная кривая — только по части в [0, 1]; всё, что ярче (портал,
    // ореолы демонов, искры душ), возвращается поверх неё нетронутым
    let sdr = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let over = color - sdr;
    color = mix(sdr, smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), sdr), k * settings.contrast) + over;

    // зерно сенсора: неподвижное в экранных координатах, множителем яркости
    let grain = hash21(floor(uv * size)) - 0.5;
    color = color * (1.0 + k * settings.grain * grain);

    return vec4<f32>(color, 1.0);
}
