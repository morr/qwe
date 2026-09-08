// Процедурная фактура кровель. Материал и смысл кодов — `src/map/buildings/material.rs`.
//
// Слой зданий несёт и крыши, и стены (в 2.5D это один меш с painter's
// порядком), поэтому первое, что делает фрагмент, — смотрит код материала:
// ноль (стена, фронтон, кайма парапета) уходит с одним вершинным цветом,
// как раньше.
//
// Фактура считается по **мировой** координате, повёрнутой в длинную ось дома
// (`meshing::ATTRIBUTE_ROOF`): швы ковра и рёбра фальца идут вдоль стен, а не
// по странам света, и при этом два треугольника одной крыши красятся
// согласованно без развёртки. Октавы гасятся по размеру пикселя (`fwidth`),
// как в `surface.wgsl`: волна короче пары пикселей исчезает, а не муарит.
//
// Посев дома (`ATTRIBUTE_ROOF.w`) несёт два смысла: фазу фактуры и **возраст**
// кровли (`roof_age`) — от него зависит, сколько на битуме заплат и насколько
// всякая кровля выгорела. Без возраста все битумные дома квартала носили ровно
// одну плотность латок.
//
// Шумовые помощники ниже — копия `surface.wgsl`; общей библиотеки шейдеров в
// проекте пока нет, а тянуть её ради четырёх функций дороже, чем повторить.

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::view,
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

// Зеркало `material::RoofParams` — порядок полей обязан совпадать.
struct RoofParams {
    light: vec2<f32>,
    intensity: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: RoofParams;

// Коды материалов — зеркало `material::RoofKind::code`; ноль это «не кровля».
const BITUMEN: u32 = 1u;
const GRAVEL: u32 = 2u;
const SEAM: u32 = 3u;
const CORRUGATED: u32 = 4u;
const TILE: u32 = 5u;
const MEMBRANE: u32 = 6u;

const TAU: f32 = 6.283185307;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_ROOF`: длинная ось дома (x, y), код материала, посев
    @location(2) roof: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec2<f32>,
    @location(1) color: vec4<f32>,
    // рамка одна на весь дом, интерполировать нечего — и код материала
    // между двумя домами интерполировать было бы просто неверно
    @location(2) @interpolate(flat) roof: vec4<f32>,
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
    out.roof = vertex.roof;
    return out;
}

// Хеш вещественной пары в [0, 1) (Dave Hoskins, hash12).
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Value noise, центрированный: [-0.5, 0.5].
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

// Видимость волны длиной `wavelength` при `px` метрах на пиксель.
fn visible(wavelength: f32, px: f32) -> f32 {
    return smoothstep(1.5, 4.0, wavelength / px);
}

// Три октавы от `scale` вниз, ~[-1, 1].
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

// Шаг сетки размещения заплат (м); доля клеток с заплатой — уже не константа,
// её даёт возраст дома (`roof_age`, `PATCH_SHARE_NEW`…`PATCH_SHARE_OLD`).
const PATCH_CELL: f32 = 6.0;
// Полуразмеры заплаты в метрах: кусок рулона, а не клетка целиком.
const PATCH_HALF_MIN: f32 = 0.9;
const PATCH_HALF_SPAN: f32 = 1.2;
// Доля клеток с заплатой у новой и у старой кровли — концы шкалы возраста.
const PATCH_SHARE_NEW: f32 = 0.04;
const PATCH_SHARE_OLD: f32 = 0.28;

// Возраст кровли — одно число на дом в [0, 1): 0 это свежий ковёр, 1 —
// латаный-перелатаный. Своего вершинного атрибута под него нет и не нужно:
// посев дома (`ATTRIBUTE_ROOF.w`) и так случаен по дому, а хеш от него даёт
// разброс, независимый от фазы швов. Корреляцию двух узоров одного дома
// глазом не увидеть, а вершина экономит четыре байта на весь слой зданий.
fn roof_age(seed: f32) -> f32 {
    return hash21(vec2<f32>(seed * 61.0 + 5.0, seed * 13.0 + 91.0));
}

// Заплата ремонта: 1 внутри прямоугольной латки, 0 снаружи, край размыт по
// пикселю. Клетка `PATCH_CELL` — только **сетка размещения**: латку получает
// меньшинство клеток (`share`), и внутри своей клетки она смещена и меньше её,
// так что соседние латки не смыкаются. Раньше здесь свой оттенок был у каждой
// клетки без исключения — это давало не заплаты, а шахматную доску во всю
// кровлю, выровненную по стенам дома. Тот же порог по доле клеток стоит на
// заплатах асфальта.
fn repair_patch(q: vec2<f32>, px: f32, seed: f32, share: f32) -> f32 {
    let cell = floor(q / PATCH_CELL);
    if hash21(cell + seed * 17.0) > share {
        return 0.0;
    }
    let half = vec2<f32>(
        PATCH_HALF_MIN + PATCH_HALF_SPAN * hash21(cell + 3.1),
        PATCH_HALF_MIN + PATCH_HALF_SPAN * hash21(cell + 7.3),
    );
    // смещение внутри клетки — ровно такое, чтобы латка не вылезала за её край
    let jitter = vec2<f32>(hash21(cell + 11.7), hash21(cell + 19.3)) - 0.5;
    let center = (cell + 0.5) * PATCH_CELL + jitter * (PATCH_CELL - 2.0 * half);
    let d = abs(q - center) - half;
    let edge = max(d.x, d.y);
    return 1.0 - smoothstep(-0.6 * px, 0.6 * px, edge);
}

// Поправка яркости кровли: сколько её фактура добавляет к вершинному цвету.
// `uv` — координаты в раме дома (вдоль длинной оси и поперёк), `across` —
// единичный вектор поперёк рёбер, `p` — мировая точка.
fn roof_shade(
    kind: u32,
    uv: vec2<f32>,
    axis: vec2<f32>,
    across: vec2<f32>,
    p: vec2<f32>,
    px: f32,
    seed: f32,
) -> f32 {
    let u = uv.x;
    let v = uv.y;
    // Возраст дома: на битуме он решает, сколько на кровле заплат и лужи, на
    // всяком материале — насколько она выгорела и замусорена. Без него квартал
    // получался одинаково заношенным: у каждой кровли поровну латок, будто их
    // крыли и ремонтировали в один год.
    let age = roof_age(seed);
    // выцветание и грязь — общее для всякой кровли, и чем дом старше, тем их
    // больше
    let weathering = mix(0.75, 1.35, age);
    var shade = weathering
        * (0.045 * fbm3(p + vec2<f32>(13.0, 29.0), 8.0, px)
            + 0.030 * fbm3(p + vec2<f32>(3.0, 7.0), 0.7, px));
    // Рулон и черепица кладутся **вдоль** конька (шов и ряд идут по длинной
    // оси, то есть при постоянном `v`), а фальц и профлист — **по скату**,
    // поперёк неё: иначе вода с крыши потечёт вдоль ребра, а не по нему.
    // Отсюда две координаты и две «поперечных» оси; свет считается по той,
    // что поперёк рёбер, — ребро вдоль солнца не даёт ни блика, ни тени.
    let slope_bite = abs(dot(axis, params.light));
    let slope_side = sign(dot(axis, params.light));
    let bite = abs(dot(across, params.light));
    let side = sign(dot(across, params.light));

    if kind == BITUMEN {
        // швы рулонов вдоль длинной оси, шаг — ширина рулона
        shade -= 0.10 * stripes(v, 0.95, 0.06, px);
        // Заплаты ремонта: свежая латка темнее выгоревшего ковра, а сколько их
        // — это возраст дома. Доля клеток идёт как `age²`, а не линейно:
        // возраст распределён ровно, и без квадрата латаных кровель в квартале
        // было бы столько же, сколько новых, тогда как чинят всё-таки
        // меньшинство. При ⟨age²⟩ = 1/3 средняя доля выходит 0.12 — вдвое
        // меньше прежних фиксированных 0.22: латаная кровля стала событием, а
        // не фоном, и заметна она теперь именно на фоне чистых соседок.
        // (`patch` — зарезервированное слово WGSL, отсюда `repair`.)
        let share = mix(PATCH_SHARE_NEW, PATCH_SHARE_OLD, age * age);
        let repair = repair_patch(vec2<f32>(u, v), px, seed, share) * visible(3.0, px);
        shade -= 0.075 * repair;
        // Застоявшаяся вода — тёмные пятна у парапета и в разжелобках; на
        // просевшей старой кровле их больше. Латку кладут как раз на протечку,
        // поэтому пятно она **закрывает**, а не складывается с ним: два
        // затемнения друг на друге дают чёрный прямоугольник посреди тёмной
        // кляксы, чего на кровле не бывает.
        let pond = value_noise(p / 4.0 + 31.0) + 0.5;
        shade -= (0.07 + 0.06 * age)
            * smoothstep(0.72, 0.86, pond)
            * visible(4.0, px)
            * (1.0 - repair);
    } else if kind == GRAVEL {
        shade += 0.12 * fbm3(p + vec2<f32>(47.0, 17.0), 0.45, px);
        let dots = value_noise(p / 0.6 + 71.0) + 0.5;
        shade += 0.07 * smoothstep(0.62, 0.80, dots) * visible(0.6, px);
    } else if kind == SEAM {
        // фальц: светлое ребро и его тень рядом, шаг — ширина картины
        let rib = stripes(u, 0.62, 0.07, px);
        let dark = stripes(u - 0.10 * slope_side, 0.62, 0.07, px);
        shade += (0.20 * rib - 0.13 * dark) * slope_bite;
    } else if kind == CORRUGATED {
        // волна профлиста: 30 см, поэтому видна только вблизи
        shade += 0.13 * cos(TAU * u / 0.30) * slope_bite * visible(0.30, px);
        // нахлёсты листов держатся дольше волны
        shade -= 0.06 * stripes(u, 1.05, 0.05, px);
    } else if kind == TILE {
        // ряды вдоль конька — тень под каждым рядом
        shade -= 0.14 * stripes(v, 0.32, 0.05, px);
        // и разнобой отдельных черепиц вдоль ряда
        shade += 0.10 * (hash21(floor(vec2<f32>(u / 0.25, v / 0.32)) + seed * 11.0) - 0.5)
            * visible(0.28, px);
    } else if kind == MEMBRANE {
        shade -= 0.05 * stripes(v, 2.0, 0.08, px);
        shade += 0.025 * fbm3(p + vec2<f32>(91.0, 5.0), 1.2, px);
    }
    return shade;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb;
    let kind = u32(round(max(in.roof.z, 0.0)));

    if kind != 0u && params.intensity > 0.0 {
        let p = in.world_position;
        // метров на пиксель; камера без поворота, так что обе производные равны
        let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
        let axis = in.roof.xy;
        let across = vec2<f32>(-axis.y, axis.x);
        let seed = in.roof.w;
        // фаза по посеву: швы соседних домов не выстраиваются в одну линию
        // через квартал
        let uv = vec2<f32>(dot(p, axis) + seed * 37.0, dot(p, across) + seed * 23.0);
        let shade = roof_shade(kind, uv, axis, across, p, px, seed);
        // тон уводится вместе с яркостью: тёмное на кровле ещё и холоднее
        let tint = vec3<f32>(0.35, 0.15, -0.30) * shade;
        rgb = rgb * (1.0 + params.intensity * (vec3<f32>(shade) + tint));
    }

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
