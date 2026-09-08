// Процедурная фактура кровель. Материал и смысл кодов — `src/map/buildings/material.rs`.
//
// Слой зданий несёт и крыши, и стены (в 2.5D это один меш с painter's
// порядком), поэтому первое, что делает фрагмент, — смотрит код материала:
// ноль (стена, фронтон, оборудование кровли) уходит с одним вершинным
// цветом, как раньше.
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

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::view,
}
#import "shaders/noise.wgsl"::{hash21, value_noise, visible, fbm3, stripes, band}

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
const WALL: u32 = 7u;
const GARAGE_ROW: u32 = 8u;
const GARAGE_BLOCK: u32 = 9u;

// Шаг бокса гаражного ряда и шаг рядов в кооперативе, м — **зеркало
// `garages::BAY` и `garages::ROW_PITCH`**. Оттуда же приходит посев: у
// прогона он не случаен, а подобран так, чтобы первый шов (у ленты
// поперечный, у кооператива проезд) лёг ровно на его край.
const BAY: f32 = 3.4;
const ROW_PITCH: f32 = 18.0;
// Из них 12 м занимают два ряда боксов спинами, остальное — проезд.
const ROW_DEPTH: f32 = 12.0;

// Стена: этаж (настоящие 3 м × `EXTRUDE_SCALE` 0.35 — столько её метра
// нарисовано), ширина панели и балкон в долях этажа. Балкон занимает нижние
// две трети этажа и половину панели по ширине — как на панельном доме.
const FLOOR: f32 = 1.05;
const PANEL: f32 = 3.2;
const BALCONY_HIGH: f32 = 0.72;
const BALCONY_LOW: f32 = 0.12;
const BALCONY_WIDE: f32 = 0.62;

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
        // Застоявшаяся вода — тёмные пятна у краёв и в разжелобках; на
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
    } else if kind == GARAGE_ROW || kind == GARAGE_BLOCK {
        // Гаражная лента. Единственное, что от неё остаётся на общем плане, —
        // поперечный шов на каждом боксе: он и делает из ленты гребёнку.
        let bay = floor(u / BAY);
        var row = 0.0;
        if kind == GARAGE_BLOCK {
            // Кооператив целиком одним контуром: под ним не ангар, а ряды
            // боксов с проездами. Проезд рисуется затемнением, а не дыркой в
            // кровле: сверху щель между двумя рядами и есть тёмная полоса, а
            // вырезать её по-настоящему значит резать контур булевой
            // операцией и разойтись со стенами и тенью.
            shade -= 0.30 * band(v, ROW_PITCH, ROW_DEPTH, ROW_PITCH - ROW_DEPTH, px);
            // и стык спина к спине посередине пары рядов
            shade -= 0.09 * stripes(v - ROW_DEPTH * 0.5, ROW_PITCH, 0.12, px);
            // номер ряда: два ряда на период, и второй начинается на
            // середине занятой боксами полосы
            let cycle = v - ROW_PITCH * floor(v / ROW_PITCH);
            row = floor(v / ROW_PITCH) * 2.0 + f32(cycle > ROW_DEPTH * 0.5);
        }
        shade -= 0.17 * stripes(u, BAY, 0.10, px);
        // каждый бокс крашен своим хозяином — ±10 % по номеру бокса (и ряда,
        // если это кооператив); тон держится ровно до шва, поэтому лента и
        // читается как ряд ворот
        shade += 0.10 * (hash21(vec2<f32>(bay, row * 13.0 + seed * 53.0)) - 0.5)
            * visible(BAY, px);
        // под швом — профлист, как и на одиночном гараже
        shade += 0.10 * cos(TAU * u / 0.30) * slope_bite * visible(0.30, px);
        // и ржавчина, которой на ГСК больше, чем на любой другой кровле
        shade -= 0.06 * fbm3(p + vec2<f32>(67.0, 41.0), 1.6, px);
    } else if kind == WALL {
        // ось стены — она сама, поэтому `v` растёт **вверх по стене**, а `u`
        // идёт вдоль неё: межэтажный шов это линия постоянного `v`
        shade -= 0.055 * stripes(v, FLOOR, 0.05, px);
        // вертикальные швы панелей
        shade -= 0.035 * stripes(u, PANEL, 0.05, px);
        // балконы: своя ячейка на этаж и панель, часть занята выступом.
        // Ячейка берётся по **обеим** координатам, так что балконы стоят
        // столбцами, как на настоящем доме, а не в шахматном порядке
        let cell = vec2<f32>(floor(u / PANEL), floor(v / FLOOR));
        let inside = fract(vec2<f32>(u / PANEL, v / FLOOR));
        let has_balcony = hash21(cell + seed * 29.0) > 0.42;
        let in_box = inside.y > BALCONY_LOW && inside.y < BALCONY_HIGH
            && abs(inside.x - 0.5) < BALCONY_WIDE * 0.5;
        // видно только вблизи: этаж на карте это доли пикселя почти всегда
        let close = visible(FLOOR, px);
        shade -= 0.10 * f32(has_balcony && in_box) * close;
        // и лёгкая грязь по стене
        shade += 0.02 * fbm3(p + vec2<f32>(57.0, 23.0), 2.0, px);
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
