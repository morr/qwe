// Процедурная фактура поверхностей карты — земля, зелень, песок, вода,
// асфальт. Материал и смысл параметров — `src/map/surface.rs`.
//
// Базовый цвет приходит вершинным, как у `ColorMaterial`; шейдер умножает его
// на шум по **мировым** координатам, поэтому две перекрывающиеся ленты одного
// слоя (стык дорог в узле) красятся в один и тот же пиксель одинаково и стык
// остаётся невидимым — то самое свойство, ради которого слой дорог плоский.
//
// Все октавы шума гасятся по размеру пикселя (`fwidth` мировой координаты):
// короткая волна не сэмплируется, а исчезает (общий `visible` из
// `shaders/noise.wgsl`, там же и порог), иначе на отдалении зерно
// превращается в муар и мерцает при движении камеры.

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_view_bindings::{view, globals},
}
#import "shaders/noise.wgsl"::{value_noise, visible, fbm3, fbm4}

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
    shore_color: vec4<f32>,
    mottle_amp: f32,
    mottle_scale: f32,
    grain_amp: f32,
    grain_scale: f32,
    speckle_amp: f32,
    speckle_scale: f32,
    speckle_threshold: f32,
    drift: f32,
    wear: f32,
    shore_width: f32,
    intensity: f32,
    // шаг полосы, м — `roads::shape::lane_width`: полоса одна по городу, и по
    // той же сетке кладёт линии слой краски (`paint.wgsl`)
    lane_width: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SurfaceParams;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_RIBBON`, м. Лента без полос (вода): поперёк ленты,
    // до разрыва (внутри разрыва отрицательно), полуширина, 0. Проезжая часть
    // с раскладкой полос (`meshing::LaneFrame`): поперёк от узла сетки полос,
    // до разрыва, нижняя и верхняя граница проезжей части от того же узла
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

// Износ асфальта. Колея — в 85 см от середины полосы (колея легковой машины
// 1.5 м); это широкая разница тона (σ 32 см, то есть около 75 см в полувысоте),
// а не тонкая линия по ширине покрышки. Амплитуда — `params.wear` (ручка Wear).
const RUT_OFFSET: f32 = 0.85;
const RUT_SIGMA: f32 = 0.32;
// Край проезжей части, на котором колея гаснет, м
const RUT_EDGE: f32 = 0.3;
// √(2π): площадь гауссианы с σ = 1, из неё доля колеи в полосе
const SQRT_TAU: f32 = 2.5066;
// На каком пути от края разрыва износ набирает силу, м. Метр, как у линий,
// читался швом: колеи обрывались поперёк полотна у самого перекрёстка
const WEAR_FADE: f32 = 5.0;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position;
    // метров на пиксель; камера без поворота, так что обе производные равны
    let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
    let k = params.intensity;

    var rgb = in.color.rgb;

    // отмель на кромках ленты русла — до всякого шума, как и вершинный цвет
    // площадной воды, который она продолжает: то же поле расстояний до берега,
    // линейно от цвета берега на краю к цвету ленты на глубине `shore_width`.
    // Узкий ручей поэтому светлый по всей ширине — мелкий, как и узкий рукав
    // полигоном. Вдоль ленты гаснет в разрыве на конце, отрезанном берегом
    // площадной воды (`map::waterways`): там вода под лентой у той же глубины
    // того же цвета. Площадная заливка ленты не несёт (полуширина ноль) — её
    // отмель лежит геометрией
    let shore_half = in.ribbon.z;
    if params.shore_width > 0.0 && shore_half > 0.0 {
        let to_edge = shore_half - abs(in.ribbon.x);
        let across = 1.0 - clamp(to_edge / params.shore_width, 0.0, 1.0);
        let along = clamp(1.0 + in.ribbon.y / params.shore_width, 0.0, 1.0);
        rgb = mix(rgb, params.shore_color.rgb, across * along);
    }

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

    // Износ покрытия — то, из-за чего асфальт на снимке никогда не ровного
    // тона: колеи под колёсами. Заплаты ремонта тут были и
    // убраны: клетка мировой сетки, залитая ровным тоном, — это шахматка по
    // сторонам света, а не заплата; см. `CONTEXT.md`.
    // Считается **в раскладке полос** (`meshing::LaneFrame`): полоса — клетка
    // сетки `params.lane_width` от её узла, внутри границ проезжей части. Та же
    // сетка у линий слоя краски, поэтому колея идёт ровно между линиями — и на
    // клине, где раскладка плывёт от узкого сечения к широкому. Раскладку
    // несёт только проезжая часть улицы; у воды на этом месте полуширина и
    // ноль (`high < low`), у полигонов нули — колеи там нет.
    let low = in.ribbon.z;
    let high = in.ribbon.w;
    if params.wear > 0.0 && high > low {
        let across = in.ribbon.x;
        // Износ гаснет в разрыве у перекрёстка. Без этого обе улицы тянут
        // свои колеи через перекрёсток, а колеи там нет: машина поперёк
        // перекрёстка едет где придётся.
        let w = k * params.wear * smoothstep(0.0, WEAR_FADE, in.ribbon.y);
        // две колеи на полосу: колёса идут в 85 см от её середины, и полоса
        // под ними отполирована до светлого
        let in_lane = across / params.lane_width;
        let from_middle = abs(in_lane - floor(in_lane) - 0.5) * params.lane_width;
        let offset = from_middle - RUT_OFFSET;
        let rut = exp(-offset * offset / (2.0 * RUT_SIGMA * RUT_SIGMA));
        // колея **без сдвига среднего**: из тона вычтена её доля по полосе
        // (два гауссиана площадью σ·√(2π) на ширину полосы). Иначе полотно с
        // износом в среднем светлее ровной улицы того же цвета, и там, где
        // проезд без полос входит в размеченную улицу — или колеи гаснут в
        // перекрёстке, — асфальт менял тон пятном
        let rut_mean = min(2.0 * RUT_SIGMA * SQRT_TAU / params.lane_width, 1.0);
        // только внутри проезжей части: у кромки колеи нет
        let inside = smoothstep(low, low + RUT_EDGE, across)
            * (1.0 - smoothstep(high - RUT_EDGE, high, across));
        // гасится по **шагу полосы**, а не по ширине колеи: рисунок повторяется
        // с полосой, и на спутниковом плане отполированные колеи ещё видны —
        // это широкая разница тона, а не тонкая линия
        rgb = rgb * (1.0 + w * (rut - rut_mean) * inside * visible(params.lane_width, px));
        // Грязи у бордюра (тёмной каймы по краю полотна) тут больше нет: разрыв
        // приходит только от перекрёстка двух проезжих частей, а проезд или
        // дворовая улица вливаются в улицу без него, и кайма широкой улицы,
        // лежащей поверх, проводила тёмную черту поперёк каждого въезда.
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
