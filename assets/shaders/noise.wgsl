// Общие помощники процедурной фактуры: хеш, value noise, октавы и полосы.
//
// До этого они были **скопированы дважды** — в `surface.wgsl` и `roof.wgsl`, —
// и копии успели разойтись комментариями, а на третьей разошлись бы и кодом.
// Импортируются по пути от `assets/`:
//
//     #import "shaders/noise.wgsl"::{hash21, value_noise, visible, fbm3}
//
// Общее у всех: **гашение по размеру пикселя**. Волна короче полутора
// пикселей не рисуется вовсе, а не муарит, — это то самое правило, из-за
// которого карту можно отдалять без ряби, и держать его в одном месте важнее,
// чем сэкономить импорт.
//
// Третий шейдер проекта, портал (`portal.wgsl`), остаётся при своих
// `hash21` / `value_noise` / `fbm` намеренно: хеш там другой, шум нецентрирован
// ([0, 1), а не [-0.5, 0.5]) и гашения по пикселю нет вовсе — вихрь считается в
// UV квада, а не по мировой позиции, и правило `visible` ему нечего держать.
// Импорт отсюда изменил бы картинку вихря, так что это решение, а не недоделка.

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

// Полоса шириной `width` через каждые `period` по координате `coord`: край
// сглажен по пикселю, сама линия не тоньше пикселя, и вся сетка гаснет,
// когда шаг становится мельче нескольких пикселей.
fn stripes(coord: f32, period: f32, width: f32, px: f32) -> f32 {
    let phase = coord - period * floor(coord / period + 0.5);
    let half_width = max(width, px) * 0.5;
    let edge = 0.6 * px;
    let line = 1.0 - smoothstep(half_width - edge, half_width + edge, abs(phase));
    return line * visible(period, px);
}
