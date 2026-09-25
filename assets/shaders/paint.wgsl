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
#import "shaders/noise.wgsl"::{visible, stripes}

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
    stop_width: f32,
    yield_dash: f32,
    yield_gap: f32,
    zebra_half: f32,
    zebra_period: f32,
    zebra_fill: f32,
    zebra_zoom: f32,
    turn_wear: f32,
    rut_offset: f32,
    rut_sigma: f32,
    lane_width: f32,
    hatch_period: f32,
    hatch_width: f32,
    edge_width: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PaintParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_RIBBON` полосы краски: поперёк от линии (м), длина
    // улицы (м), до разрыва перекрёстка (м), вид линии (0 — линия полос,
    // 1 — осевая, 2 — двойная сплошная, 3 — стоп-линия, 4 — она же
    // прерывистой, 5 — зебра, 6 — колея траектории узла, 7 — штриховка
    // островка, 8 — его обводка, 9 — стрелка, 10 и 11 — линия полос
    // пунктиром и сплошной подхода, 12 и 13 — осевая пунктиром и сплошной). У поперечной краски (3–5) «длина» идёт
    // поперёк дороги от кромки, а «поперёк» — вдоль неё; у штриховки
    // «длина» — координата поперёк её косых полос. Колея траектории рисуется
    // своими проходами (`WEAR_MASK`, `WEAR_APPLY`), её сила — в альфе вершины
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
#ifdef WEAR_APPLY
    // наложение колеи (`roads::paint::PaintPass::Wear`): смешивание берёт
    // цвет кадра × (единица + (1 − альфа маски)) и возвращает альфу в
    // единицу — от шейдера нужна только единица
    return vec4<f32>(1.0);
#else
    let p = in.world_position;
    let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
    let across = in.ribbon.x;
    let along = in.ribbon.y;
    let to_break = in.ribbon.z;
    let kind = u32(round(in.ribbon.w));

#ifdef WEAR_MASK
    // маска колеи (`PaintPass::WearMask`): две колеи по сторонам кривой,
    // профиль колеи полос (`surface.wgsl`), сила в альфе вершины (хвост сходит
    // в ноль). Пишется `1 − колея` в альфу кадра операцией `min` — остаётся
    // наибольшая колея из всех полос. Гаснет по шагу полосы, как колея
    // асфальта, и к порогу осевых, где меш прячется
    let offset = abs(across) - params.rut_offset;
    let rut = exp(-offset * offset / (2.0 * params.rut_sigma * params.rut_sigma));
    let far = 1.0 - smoothstep(params.axis_zoom * params.fade_from, params.axis_zoom, px);
    let wear = params.turn_wear * in.color.a * rut * visible(params.lane_width, px) * far;
    return vec4<f32>(0.0, 0.0, 0.0, 1.0 - wear);
#else
    var cover = 0.0;
    if kind == 7u {
        // штриховка островка: заливка контура, полосы — по координате поперёк
        // них; мельче пары пикселей гаснут в свою среднюю долю, как зебра
        let fill = params.hatch_width / params.hatch_period;
        let bars = stripes(along, params.hatch_period, params.hatch_width, px);
        let seen = visible(params.hatch_period, px);
        cover = bars + fill * (1.0 - seen);
    } else if kind == 8u {
        cover = line_cover(abs(across), params.edge_width, px);
    } else if kind == 9u {
        // стрелка на полосе: заливка контура целиком
        cover = 1.0;
    } else if kind == 5u {
        // зебра: плашка вдоль дороги, полосы поперёк неё; где период мельче
        // пары пикселей, полосы гаснут в свою среднюю долю — светлую плашку
        let edge = 0.7 * px;
        let body = 1.0 - smoothstep(params.zebra_half - edge, params.zebra_half + edge, abs(across));
        let bars = stripes(
            along - params.zebra_period * 0.5,
            params.zebra_period,
            params.zebra_period * params.zebra_fill,
            px,
        );
        let seen = visible(params.zebra_period, px);
        cover = body * (bars + params.zebra_fill * (1.0 - seen));
    } else if kind == 3u || kind == 4u {
        // стоп-линия; у «уступи дорогу» — штрихами поперёк дороги
        cover = line_cover(abs(across), params.stop_width, px);
        if kind == 4u {
            let edge = 0.7 * px;
            let dash = dash_distance(along, params.yield_dash, params.yield_gap);
            let dashed = 1.0 - smoothstep(-edge, edge, dash);
            let mean = params.yield_dash / (params.yield_dash + params.yield_gap);
            cover = cover * mix(mean, dashed, visible(params.yield_dash + params.yield_gap, px));
        }
    } else if kind == 2u {
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
        // на открытой улице сплошной кусок отмерен геометрией: у линии полос
        // по ходу её полос (10 — пунктир, 11 — подход к узлу), у осевой по
        // обе стороны разрыва и у узла насквозь (12 — пунктир, 13 —
        // сплошная); на кольце — по близости разрыва
        var solid = 1.0 - smoothstep(params.approach - 1.0, params.approach, to_break);
        if kind == 10u || kind == 12u {
            solid = 0.0;
        } else if kind == 11u || kind == 13u {
            solid = 1.0;
        }
        cover = cover * mix(seen, 1.0, solid);
    }
    // в разрыве перекрёстка линии нет; у края разрыва она обрывается резко —
    // торец сглажен на те же 1.4 px, что и её бока. Начинается линия в 20 см
    // от края: между двумя разрывами, что сходятся встык, остаётся щель в
    // сантиметры (разрыв проецируется на свою ось заново), и в ней торчал
    // обрезок линии — метровое гашение его прятало, резкий торец показал
    let cut = 1.4 * px;
    cover = cover * smoothstep(0.2, 0.2 + cut, to_break);
    // с зумом: линии полос уходят раньше осевых; к порогу, где меш прячется
    // (`roads::paint::PaintLods`), линия уже прозрачна
    var zoom_max = params.axis_zoom;
    if kind == 0u || kind == 3u || kind == 4u || kind == 9u || kind == 10u || kind == 11u {
        zoom_max = params.lane_zoom;
    } else if kind == 5u || kind == 7u || kind == 8u {
        zoom_max = params.zebra_zoom;
    }
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
#endif
#endif
}
