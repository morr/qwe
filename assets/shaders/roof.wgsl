// Процедурная фактура кровель. Материал и смысл кодов — `src/map/buildings/material.rs`.
//
// Слой зданий несёт и крыши, и стены (в 2.5D это один меш с painter's
// порядком), поэтому первое, что делает фрагмент, — смотрит код материала:
// ноль (оборудование кровли) уходит с одним вершинным цветом, коды `1…6` идут
// кровельным трактом, `7…11` — стенным. Стен несколько, и это не оттенки
// одного: панель, кирпич, штукатурка, витраж и профлист отличаются и тем, что
// между окнами, и самими окнами — а балкон бывает только у первых двух.
//
// Окно — единственное здесь, что **заменяет** цвет поверхности, а не
// поправляет его: стекло не бывает штукатуркой на столько-то процентов темнее.
// Оттого стена и отдаёт фрагменту три числа (`Wall`), а не одну яркость.
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

// Коды материалов — **один словарь на кровлю и стену**, зеркало
// `material::RoofKind::code` и `material::WallKind::code`. Ноль это «фактуры
// нет вовсе»: оборудование кровли, кайма.
const BITUMEN: u32 = 1u;
const GRAVEL: u32 = 2u;
const SEAM: u32 = 3u;
const CORRUGATED: u32 = 4u;
const TILE: u32 = 5u;
const MEMBRANE: u32 = 6u;
const PANEL: u32 = 7u;
const BRICK: u32 = 8u;
const PLASTER: u32 = 9u;
const SHOPFRONT: u32 = 10u;
const SHED: u32 = 11u;

// В том же числе, что и код, едет **число этажей** стены: код в остатке от
// деления, этажи в частном (`meshing::STOREY_STRIDE` — зеркало). Без них
// шейдер знал только низ стены и не знал верха, а верх это карниз: верхнее
// окно упиралось прямо в кровлю.
const STOREY_STRIDE: u32 = 16u;

const TAU: f32 = 6.283185307;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // `meshing::ATTRIBUTE_ROOF`: у кровли длинная ось дома (x, y), у стены её
    // собственные координаты (номер панели, этаж); дальше код материала и посев
    @location(2) roof: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec2<f32>,
    @location(1) color: vec4<f32>,
    // Интерполируется, и это обязательно: координаты стены у каждой вершины
    // свои, фрагменту нужно значение в его точке. Кровле интерполяция ничего
    // не портит — у всех вершин дома тут одно и то же число, — а код материала
    // и посев одинаковы у всех вершин **любой** грани, так что между двумя
    // домами усредняться нечему: треугольник целиком принадлежит одному.
    @location(2) roof: vec4<f32>,
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

// ─── стена ──────────────────────────────────────────────────────────────────
//
// Всё, что ниже, считается **в долях ячейки**, а не в метрах: `roof.xy` у
// стены это номер панели и этаж (`meshing::WallFrame`), поэтому ни ширины
// панели, ни высоты этажа, ни косины подъёма шейдеру знать не нужно — только
// дробная часть. Сколько метров в ячейке, решил сборщик, и уложил в стену
// **целое** число панелей и этажей: у края стены не бывает обрезанной панели,
// под карнизом — полуэтажа, а окно или балкон, сидящий внутри своей ячейки, ни
// в один край стены не упирается. Раньше сетка была глобальная, в метрах, и
// делала ровно это: резала проёмы по краям стены.

// Окно панельного дома: одно на панель, широкое, с импостом посередине.
const WINDOW_LOW: f32 = 0.30;
const WINDOW_HIGH: f32 = 0.72;
const WINDOW_WIDE: f32 = 0.42;
// Кирпичное окно уже панельного: проём в кладке дорог, и его не расширяют.
const BRICK_WINDOW_WIDE: f32 = 0.32;
// Частный дом: окно мелкое, сидит выше и занимает четверть простенка.
const PLASTER_WINDOW_LOW: f32 = 0.34;
const PLASTER_WINDOW_HIGH: f32 = 0.72;
const PLASTER_WINDOW_WIDE: f32 = 0.26;
// Витраж: остекление почти во всю панель, лентой через этаж.
const SHOPFRONT_LOW: f32 = 0.18;
const SHOPFRONT_HIGH: f32 = 0.76;
const SHOPFRONT_WIDE: f32 = 0.90;
// Профлист: ленточное окно высоко под карнизом, и то не на всякой панели.
//
// Лента высокая нарочно. Всё, что на стене **ниже трети ячейки**, на рабочем
// зуме не доживает: подъём сжимает стену втрое по высоте, этаж выходит около
// семи пикселей, а `visible` гасит волну короче полутора из них. Первая версия
// была втрое ниже (0.60…0.82) — и склад стоял глухой коробкой, единственный из
// пяти облицовок вовсе без рисунка.
const SHED_WINDOW_LOW: f32 = 0.45;
const SHED_WINDOW_HIGH: f32 = 0.76;
const SHED_WINDOW_WIDE: f32 = 0.76;
const SHED_WINDOW_SHARE: f32 = 0.55;

// Рама проёма, импост, отлив под окном и тень откоса под перемычкой.
const FRAME_WIDTH: f32 = 0.030;
const MULLION_WIDTH: f32 = 0.018;
const SILL_HEIGHT: f32 = 0.040;
const REVEAL_HEIGHT: f32 = 0.070;

// Карниз — светлая полоса по самому верху стены, доля **верхнего** этажа.
// Стена обязана чем-то кончаться: под ней земля и цоколь, над ней кровля и
// парапет, и без него верхнее окно упирается в кровлю впритык. Поэтому ни
// один проём выше `1 - PARAPET_HIGH` не поднимается — верх у всех у них 0.80
// или ниже, с запасом на раму.
const PARAPET_HIGH: f32 = 0.20;
// Период для `stripes`, когда линия нужна ровно одна: заведомо больше любой
// стены в этажах, так что вторая никуда не попадает.
const FAR_APART: f32 = 1000.0;

// Балкон снизу вверх: тень плиты на стене, торец плиты, ограждение, а над ним
// либо остекление, либо открытый провал в тени.
const BALCONY_WIDE: f32 = 0.72;
const BALCONY_SLAB_LOW: f32 = 0.07;
const BALCONY_SLAB_HIGH: f32 = 0.15;
const BALCONY_RAIL_HIGH: f32 = 0.46;
const BALCONY_HIGH: f32 = 0.74;
// Доля столбцов с балконом и доля остеклённых среди них.
const BALCONY_SHARE: f32 = 0.58;
const BALCONY_GLAZED: f32 = 0.62;

// Цоколь — доля первого этажа под ним.
const PLINTH_HIGH: f32 = 0.14;

// Вход на первом этаже: доля столбцов, ширина и высота проёма.
const DOOR_SHARE: f32 = 0.24;
const DOOR_WIDE: f32 = 0.30;
const DOOR_HIGH: f32 = 0.60;
// Ворота склада — шире двери и **почти квадратные**: доли тут в разных
// единицах, ширина в панелях (≈3.2 м), высота в этажах (3 м), поэтому 0.72 на
// 0.78 это примерно 2.3 × 2.3 м — гаражные ворота или небольшие складские.
// Первая версия была 0.62 на 0.50, то есть 2.0 м в ширину при 1.5 м в высоту:
// шире, чем выше, чего не бывает ни у одних ворот и ни у одной двери. На
// экране это тем заметнее, что подъём сжимает стену по высоте втрое, и проём
// читался щелью почтового ящика.
const GATE_SHARE: f32 = 0.30;
const GATE_WIDE: f32 = 0.72;
const GATE_HIGH: f32 = 0.78;

// Швы панели, ряд кирпичной кладки и ребро профлиста — тоже доли ячейки.
const FLOOR_SEAM: f32 = 0.05;
const PANEL_SEAM: f32 = 0.016;
const BRICK_COURSE: f32 = 0.0833;
// Ребро профлиста — восьмая часть панели, то есть около 40 см. Настоящий
// профнастил ребрится вдвое чаще, но восьмушка панели это ~2.5 пикселя на
// рабочем зуме, а шестнадцатая — 1.4, ниже порога `visible`, и стена от рёбер
// не получала ничего.
const SHED_RIB: f32 = 0.125;

// Цвета стекла — **линейные**, потому что вершинный цвет здесь линейный
// (`wall_colors` отдаёт `LinearRgba`). В sRGB это примерно 0.13/0.15/0.18 у
// тёмного и 0.62/0.71/0.80 у светлого: тёмная комната за стеклом и отражённое
// в нём небо. Между ними и ходит окно — снизу комната, сверху небо.
const GLASS_ROOM: vec3<f32> = vec3<f32>(0.015, 0.020, 0.027);
const GLASS_SKY: vec3<f32> = vec3<f32>(0.342, 0.462, 0.604);

// Что стена делает с пикселем. Яркость и стекло разведены намеренно: шов и
// тень — это **поправка** к цвету стены, а окно её цветом не является вовсе,
// им стена заменяется.
struct Wall {
    // поправка яркости самой стены: швы, тени, торцы плит
    shade: f32,
    // сколько в пикселе стекла
    glass: f32,
    // чего в этом стекле больше: неба (1) или тёмной комнаты (0)
    sky: f32,
}

// Полоса по одной координате: 1 между `lo` и `hi`, края размыты по пикселю.
fn cell_band(t: f32, lo: f32, hi: f32, px: f32) -> f32 {
    let edge = 0.6 * px;
    return smoothstep(lo - edge, lo + edge, t) * (1.0 - smoothstep(hi - edge, hi + edge, t));
}

// Проём со стеклом. Рама и отлив идут в яркость, само стекло — в `glass`, а
// `sky` растёт кверху: внизу окна видно тёмную комнату, вверху — отражённое
// небо, и под самой перемычкой его съедает тень откоса.
//
// `tone` — своё у каждого окна: занавеска, открытая створка, немытое стекло.
// Без него ряд окон читается как трафарет, а не как дом.
//
// `panes` — на сколько створок делит проём переплёт; импосты ставятся от
// левого края проёма, поэтому число створок может быть любым, а крайние линии
// приходятся ровно на раму и в ней теряются.
fn window_of(
    inside: vec2<f32>,
    px: vec2<f32>,
    lo: f32,
    hi: f32,
    wide: f32,
    panes: f32,
    tone: f32,
) -> Wall {
    var out = Wall(0.0, 0.0, 0.0);
    // Проём гаснет по **своему** размеру, а не по размеру ячейки: на общем
    // плане от стены остаётся её цвет, и дырок в нём быть не должно.
    let seen = visible(hi - lo, px.y) * visible(wide, px.x);
    if seen <= 0.0 {
        return out;
    }
    let half = wide * 0.5;
    let pane = cell_band(inside.x, 0.5 - half, 0.5 + half, px.x)
        * cell_band(inside.y, lo, hi, px.y);
    // откос — светлая полоса вокруг проёма
    let outer = cell_band(inside.x, 0.5 - half - FRAME_WIDTH, 0.5 + half + FRAME_WIDTH, px.x)
        * cell_band(inside.y, lo - FRAME_WIDTH, hi + FRAME_WIDTH, px.y);
    out.shade += 0.05 * (outer - pane) * seen;
    // отлив под окном: подоконный слив всегда светлее стены под ним
    out.shade += 0.07
        * cell_band(inside.x, 0.5 - half - FRAME_WIDTH, 0.5 + half + FRAME_WIDTH, px.x)
        * cell_band(inside.y, lo - FRAME_WIDTH - SILL_HEIGHT, lo - FRAME_WIDTH, px.y)
        * seen;
    // переплёт: импосты режут стекло, поэтому вычитаются из него
    let bars = stripes(inside.x - 0.5 + half, wide / panes, MULLION_WIDTH, px.x);
    out.glass = pane * (1.0 - bars) * seen;
    // небо сверху, комната снизу, тень откоса под самой перемычкой
    let up = clamp((inside.y - lo) / max(hi - lo, 1e-3), 0.0, 1.0);
    let reveal = 1.0 - 0.7 * cell_band(inside.y, hi - REVEAL_HEIGHT, hi + 1.0, px.y);
    out.sky = clamp(mix(0.10, 0.85, up) * tone * reveal, 0.0, 1.0);
    return out;
}

// Балкон одной ячейки. Столбцом решает вызывающий — здесь только рисунок:
// тень плиты на стене, светлый торец плиты, ограждение и то, что над ним.
//
// Читается он прежде всего **полосами**: столбец балконов это стопка
// чередующихся светлых и тёмных лент, и она видна раньше, чем становится
// видно, что там внутри.
//
// `recessed` — лоджия вместо выступа: у кирпичного дома балкон утоплен в
// стену, у него нет вылета плиты, зато глубже тень.
fn balcony_of(inside: vec2<f32>, px: vec2<f32>, glazed: bool, recessed: bool, tone: f32) -> Wall {
    var out = Wall(0.0, 0.0, 0.0);
    let seen = visible(BALCONY_HIGH - BALCONY_SLAB_LOW, px.y) * visible(BALCONY_WIDE, px.x);
    if seen <= 0.0 {
        return out;
    }
    let half = BALCONY_WIDE * 0.5;
    let across = cell_band(inside.x, 0.5 - half, 0.5 + half, px.x);
    // тень плиты на стене под балконом — у лоджии её нет, вылета-то нет
    let overhang = select(0.10, 0.0, recessed);
    out.shade -= overhang * across
        * cell_band(inside.y, BALCONY_SLAB_LOW - 0.06, BALCONY_SLAB_LOW, px.y)
        * seen;
    // торец плиты — самая светлая полоса балкона
    out.shade += select(0.10, 0.03, recessed) * across
        * cell_band(inside.y, BALCONY_SLAB_LOW, BALCONY_SLAB_HIGH, px.y)
        * seen;
    // ограждение: от светлой панели до тёмного профлиста, по своему броску
    out.shade += (0.07 - 0.16 * tone) * across
        * cell_band(inside.y, BALCONY_SLAB_HIGH, BALCONY_RAIL_HIGH, px.y)
        * seen;
    let upper = across * cell_band(inside.y, BALCONY_RAIL_HIGH, BALCONY_HIGH, px.y) * seen;
    if glazed {
        // Створок две, а не четыре. Подъём сжимает стену по высоте втрое
        // (`EXTRUDE_SCALE`), и балкон на экране — лента вчетверо шире своей
        // высоты: разрезанная на четыре части, она читается россыпью точек, а
        // не балконом. Балкон обязан читаться **полосами** — светлый торец
        // плиты, ограждение, стекло, — и всё, что рубит ленту поперёк, эти
        // полосы и съедает.
        let bars = stripes(inside.x - 0.5 + half, BALCONY_WIDE / 2.0, MULLION_WIDTH, px.x);
        out.glass = upper * (1.0 - bars);
        // остеклённый балкон светлее окна: стекло у него ближе к плоскости
        // стены и ловит небо целиком, а тёмной комнаты за ним нет вовсе
        out.sky = clamp(0.45 + 0.40 * tone, 0.0, 1.0);
    } else {
        // открытый балкон — провал в тени под собственной плитой
        out.shade -= (0.14 + 0.06 * f32(recessed)) * upper;
    }
    return out;
}

// Тёмный проём без неба: подъезд, ворота склада. Небо в нём не отражается —
// вход всегда в глубине, под козырьком, — зато над ним самим козырёк и есть.
fn doorway_of(inside: vec2<f32>, px: vec2<f32>, wide: f32, high: f32) -> Wall {
    var out = Wall(0.0, 0.0, 0.0);
    let seen = visible(high, px.y) * visible(wide, px.x);
    if seen <= 0.0 {
        return out;
    }
    let half = wide * 0.5;
    let across = cell_band(inside.x, 0.5 - half, 0.5 + half, px.x);
    out.glass = across * cell_band(inside.y, 0.02, high, px.y) * seen;
    out.sky = 0.08;
    // козырёк над входом
    out.shade += 0.08
        * cell_band(inside.x, 0.5 - half - FRAME_WIDTH, 0.5 + half + FRAME_WIDTH, px.x)
        * cell_band(inside.y, high, high + SILL_HEIGHT, px.y)
        * seen;
    return out;
}

// Что стена делает с пикселем: сперва **рисунок материала** между проёмами —
// швы панели, ряды кладки, разводы штукатурки, рёбра профлиста, — потом сам
// проём. Проёмы у одного пикселя не спорят: в ячейке стоит либо балкон, либо
// окно, либо вход, и выбор между ними и есть то, чем один тип дома отличается
// от другого.
//
// Стена считается отдельно от кровли и уходит из фрагмента раньше — это не
// стилистика, а её цена. `roof_age` — возраст **кровли**, у стены его нет; а
// общая октава кровельного тракта в 8 м гаснет только к `px ≈ 5 м/пиксель`,
// то есть уводила бы цвет стен и на общем плане, где фактура уже погасла.
fn wall_shade(kind: u32, cell: vec2<f32>, storeys: f32, seed_raw: f32) -> Wall {
    // Метка поверхности едет посевом (`meshing::WallFrame::encoded_seed`):
    // `[0, 1)` — стена с балконами, `(-2, -1]` — без них, `(-4, -3]` — фронтон.
    // Шейдеру этого не вывести самому: он не знает назначения дома — решает
    // `buildings::layers::balconies_fit`.
    let gable = seed_raw < -2.5;
    let blank = seed_raw < 0.0;
    let seed = select(seed_raw, select(-seed_raw - 1.0, -seed_raw - 3.0, gable), blank);
    // Ячеек на пиксель — прямо из производной координаты. Ракурс сжимает стену
    // тем сильнее, чем круче она развёрнута, и `fwidth` уже знает об этом всё,
    // что нужно: мерить сжатие отдельно, через косину подъёма, не надо.
    let px = vec2<f32>(max(fwidth(cell.x), 1e-4), max(fwidth(cell.y), 1e-4));
    // «крупно ли видно ячейку» — по большей из двух производных: по одной оси
    // ракурс может сжать стену вдвое, и рисунок обязан гаснуть по худшей
    let coarse = max(px.x, px.y);
    let column = floor(cell.x);
    let storey = floor(cell.y);
    let inside = fract(cell);

    var out = Wall(0.0, 0.0, 0.0);

    // 1. что между проёмами — сам материал
    if kind == PANEL {
        // межэтажный шов — линия постоянного этажа, параллельная карнизу
        out.shade -= 0.055 * stripes(cell.y, 1.0, FLOOR_SEAM, px.y);
        // шов панели — параллельная боковым рёбрам стены
        out.shade -= 0.035 * stripes(cell.x, 1.0, PANEL_SEAM, px.x);
        // каждая плита отлита и выкрашена отдельно, и на стене это видно; шаг
        // клеток тут не порок, а сама вещь — стена и правда собрана из плит
        out.shade += 0.020 * (hash21(vec2<f32>(column, storey) + seed * 7.0) - 0.5)
            * visible(1.0, coarse);
    } else if kind == BRICK {
        // ряды кладки: мелкие, поэтому гаснут первыми и остаётся ровный тон
        out.shade -= 0.030 * stripes(cell.y, BRICK_COURSE, BRICK_COURSE * 0.30, px.y);
        out.shade += 0.012 * (hash21(vec2<f32>(column, storey) + seed * 7.0) - 0.5)
            * visible(1.0, coarse);
    } else if kind == PLASTER {
        // штукатурка: разводы и подтёки, без единого шва
        out.shade += 0.040 * fbm3(cell + seed * 19.0, 0.9, coarse);
    } else if kind == SHOPFRONT {
        // глухой пояс между лентами остекления
        out.shade -= 0.045 * stripes(cell.y, 1.0, 0.10, px.y);
    } else if kind == SHED {
        // рёбра профлиста во всю стену — единственный её рисунок
        out.shade += 0.055 * cos(TAU * cell.x / SHED_RIB) * visible(SHED_RIB, px.x);
        out.shade -= 0.030 * stripes(cell.y, 1.0, 0.03, px.y);
    }

    // Фронтон — верх той же стены, и рисунок материала на нём продолжается,
    // а проёмов нет: окно на треугольнике резалось бы скатом.
    if gable {
        return out;
    }

    // 2. проём. Первый этаж живёт по своим правилам у всякой стены: балкона на
    // нём не бывает, зато бывает вход — и это первое, чем низ дома отличается
    // от его середины.
    let ground = storey < 0.5;

    // Цоколь — тёмная полоса по самому низу первого этажа. Он есть у всякого
    // дома, из чего бы тот ни был, и это единственная линия здесь, которая
    // говорит, где дом **кончается и начинается земля**: без неё светлая стена
    // упирается в светлую землю карты, и низ коробки теряется. Полоса идёт
    // поперёк стены на всю её длину, поэтому гаснет по высоте ячейки, а не по
    // своей собственной — иначе на общем плане исчезла бы первой.
    out.shade -= 0.09 * f32(ground) * cell_band(inside.y, -1.0, PLINTH_HIGH, px.y)
        * visible(1.0, px.y);

    // Карниз — то же самое на другом конце стены, и появился он позже цоколя
    // ровно потому, что верх шейдеру был неизвестен: `storeys` едет в слоте
    // кода (`STOREY_STRIDE`) как раз ради этой полосы. Без неё верхнее окно
    // упиралось в кровлю впритык, чего на снимке не бывает: над последним
    // этажом всегда есть перекрытие и парапет.
    //
    // Полоса светлая: парапет стоит **над** линией кровли и ловит свет сверху.
    // Но светлое на светлой стене само по себе не читается, и работает тут не
    // полоса, а **шов под ней** — граница, ниже которой начинаются этажи.
    //
    // Шов идёт через `stripes`, а не через `cell_band`, и это не вкус: `stripes`
    // держит линию не тоньше пикселя (`max(width, px)`), а `cell_band` — нет, и
    // шов в 0.05 ячейки (5 см нарисованных) гас начисто на всяком зуме, где
    // стену вообще видно. Период заведомо больше любой стены, поэтому линия
    // одна: та, что приходится на низ парапета.
    // Амплитуда втрое больше межэтажного шва (0.055) намеренно: конец стены —
    // событие крупнее, чем стык двух этажей, и на глаз должен быть крупнее.
    let below_eaves = storeys - cell.y;
    out.shade += 0.14 * cell_band(below_eaves, -1.0, PARAPET_HIGH, px.y) * visible(1.0, px.y);
    out.shade -= 0.20 * stripes(below_eaves - PARAPET_HIGH, FAR_APART, FLOOR_SEAM, px.y);
    // свой бросок на ячейку: занавеска в окне, остекление балкона, тон
    // ограждения. По ячейке, а не по столбцу, — в отличие от самого балкона.
    let tone = hash21(vec2<f32>(column, storey) + seed * 53.0);
    let entrance = hash21(vec2<f32>(column, 7.0) + seed * 71.0);

    if kind == PANEL || kind == BRICK {
        let recessed = kind == BRICK;
        // Бросок идёт по **номеру панели**, а не по всей ячейке: хеш от обеих
        // координат — независимый бросок на каждую, то есть шахматный порядок,
        // а на жилом доме балконы стоят столбцом во всю высоту.
        let column_share = select(BALCONY_SHARE, BALCONY_SHARE * 0.62, recessed);
        let balcony_here = !blank && !ground
            && hash21(vec2<f32>(column, 0.0) + seed * 29.0) < column_share;
        let wide = select(WINDOW_WIDE, BRICK_WINDOW_WIDE, recessed);
        if balcony_here {
            let b = balcony_of(inside, px, tone < BALCONY_GLAZED, recessed, tone);
            out.shade += b.shade;
            out.glass = b.glass;
            out.sky = b.sky;
        } else if ground && entrance < DOOR_SHARE {
            let d = doorway_of(inside, px, DOOR_WIDE, DOOR_HIGH);
            out.shade += d.shade;
            out.glass = d.glass;
            out.sky = d.sky;
        } else {
            let w = window_of(inside, px, WINDOW_LOW, WINDOW_HIGH, wide, 2.0, 0.55 + 0.75 * tone);
            out.shade += w.shade;
            out.glass = w.glass;
            out.sky = w.sky;
        }
    } else if kind == PLASTER {
        // Частный дом: окно мелкое, и оно не на всякой панели — простенок
        // между окнами тут шире самого окна.
        if ground && entrance < DOOR_SHARE {
            let d = doorway_of(inside, px, DOOR_WIDE * 0.8, DOOR_HIGH);
            out.shade += d.shade;
            out.glass = d.glass;
            out.sky = d.sky;
        } else {
            let w = window_of(
                inside,
                px,
                PLASTER_WINDOW_LOW,
                PLASTER_WINDOW_HIGH,
                PLASTER_WINDOW_WIDE,
                2.0,
                0.45 + 0.8 * tone,
            );
            out.shade += w.shade;
            out.glass = w.glass;
            out.sky = w.sky;
        }
    } else if kind == SHOPFRONT {
        // Первый этаж торгового дома — витрина: ниже и выше обычной ленты.
        let lo = select(SHOPFRONT_LOW, 0.08, ground);
        let hi = select(SHOPFRONT_HIGH, 0.78, ground);
        let w = window_of(inside, px, lo, hi, SHOPFRONT_WIDE, 3.0, 0.75 + 0.4 * tone);
        out.shade += w.shade;
        out.glass = w.glass;
        out.sky = w.sky;
    } else if kind == SHED {
        // Склад: ворота на первом этаже, ленточное окно под карнизом.
        //
        // **Ровно один проём на ячейку**, как и у прочих облицовок. Раньше эти
        // два рисовались независимо, «потому что стоят на разной высоте», — и
        // это перестало быть правдой в тот момент, когда ленту опустили с 0.60
        // до 0.45, чтобы она пережила гашение. Створки окна лезли на воротное
        // полотно, и низ ленты выходил обрубком поверх тёмного прямоугольника.
        // Разной высоты мало: проёму принадлежит ещё и откос с отливом под ним,
        // так что зазор между двумя должен быть не нулевым, а с запасом на раму
        // — надёжнее не оставлять зазора вовсе, а выбирать один из двух.
        let gate_here = ground && hash21(vec2<f32>(column, 3.0) + seed * 97.0) < GATE_SHARE;
        if gate_here {
            let d = doorway_of(inside, px, GATE_WIDE, GATE_HIGH);
            out.shade += d.shade;
            out.glass = d.glass;
            out.sky = d.sky;
        } else if entrance < SHED_WINDOW_SHARE {
            let w = window_of(
                inside,
                px,
                SHED_WINDOW_LOW,
                SHED_WINDOW_HIGH,
                SHED_WINDOW_WIDE,
                4.0,
                0.5 + 0.5 * tone,
            );
            out.shade += w.shade;
            out.glass = w.glass;
            out.sky = w.sky;
        }
    }
    return out;
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
    }
    return shade;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb;
    // Слот несёт два числа: код материала в остатке и, у стены, число её
    // этажей в частном (`meshing::unpack_material` — зеркало). У кровли и у
    // каймы частное ноль, и деление им ничего не портит.
    let packed = u32(round(max(in.roof.z, 0.0)));
    let kind = packed % STOREY_STRIDE;
    let storeys = f32(packed / STOREY_STRIDE);

    if kind != 0u && params.intensity > 0.0 {
        var shade = 0.0;
        // сколько в пикселе стекла и какого — у кровли ни того, ни другого
        var glass = 0.0;
        var sky = 0.0;
        if kind >= PANEL {
            // у стены координаты уже свои, в вершине; мировая точка, ось дома
            // и фаза по посеву ей не нужны вовсе
            let wall = wall_shade(kind, in.roof.xy, storeys, in.roof.w);
            shade = wall.shade;
            glass = wall.glass;
            sky = wall.sky;
        } else {
            let p = in.world_position;
            // метров на пиксель; камера без поворота, так что обе производные
            // равны
            let px = max(max(fwidth(p.x), fwidth(p.y)), 1e-4);
            let axis = in.roof.xy;
            let across = vec2<f32>(-axis.y, axis.x);
            let seed = in.roof.w;
            // фаза по посеву: швы соседних домов не выстраиваются в одну линию
            // через квартал
            let uv = vec2<f32>(dot(p, axis) + seed * 37.0, dot(p, across) + seed * 23.0);
            shade = roof_shade(kind, uv, axis, across, p, px, seed);
        }
        // тон уводится вместе с яркостью: тёмное на кровле ещё и холоднее
        let tint = vec3<f32>(0.35, 0.15, -0.30) * shade;
        rgb = rgb * (1.0 + params.intensity * (vec3<f32>(shade) + tint));
        // Окно — не поправка к цвету стены, а **замена** его: стекло не бывает
        // светлее или темнее штукатурки на столько-то процентов, оно другого
        // цвета вовсе. Отсюда `mix`, а не множитель; ползунок фактуры при этом
        // остаётся одним на всё — на нуле стена снова ровная заливка.
        rgb = mix(rgb, mix(GLASS_ROOM, GLASS_SKY, sky), params.intensity * glass);
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
