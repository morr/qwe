//! Слой дорог, аллей и стен Кремля: по ленте на `RoadLine`/`WallLine`, слитой
//! в merged-меш на класс. Стиль ленты — ресурс [`RoadStyle`], переключаемый на
//! лету панелью Roads (`ui/roads.rs`); правка пересобирает только эти слои
//! ([`rebuild_roads`]). Рельсовые пути — в `map/rail.rs`, трамвай — в
//! `map/tram.rs`: у обоих свой стиль и свой зум-LOD, и пересобираются они по
//! зуму, а не по [`RoadStyle`].
//!
//! Мост (`RoadLine::bridge`) уходит из слоёв своего класса в тройку
//! `bridge_shadows` + `bridge_casings` + `bridges`: серый бордюр по краям
//! настила (всегда, вне зависимости от `RoadStyle::casing`) и заливка цветом
//! класса над `Z_ROAD` — эстакада кроет улицу, которую пересекает, а ровные
//! торцы бордюра читаются как края настила, вид 2ГИС. Под ними —
//! [`push_bridge_shadow`]: единственное на карте, что говорит, что настил
//! поднят, потому что наземные тени считают только дома.
//!
//! Раньше дороги рисовал `MeshBuilder::push_polyline` — свой квад на
//! сегмент, продлённый с обоих концов на полуширины. Стыков у него нет вообще:
//! на изломе продление торчит за внешний угол прямоугольным выступом, между
//! двумя выступами остаётся выемка, а торец пути — квадратный шип. Дефолт
//! теперь [`RoadJoin::Round`] — дуга на внешней стороне излома и полудиск на
//! торце, то же самое, что `stroke-linejoin: round` + `stroke-linecap: round`
//! у Mapnik, которым нарисован osm-carto: круглые торцы двух ways в общем узле
//! перекрываются и сливаются в скруглённый стык.
//!
//! Осевая (`RoadLine::points`) при этом **не трогается**: на ней стоят навмеш
//! (`bridge`/`passage`-прорезы), арки, посадка деревьев и генератор дверей.
//! Chaikin-сглаживание работает на копии и только ради картинки.
//!
//! Улица — это не одна лента, а три слоя: **тротуар** (`Z_SIDEWALK`, светлая
//! полоса шире проезжей части на [`sidewalk_width`] с каждой стороны), кант и
//! заливка асфальтом. Тротуар лежит под всеми лентами дорог по той же логике,
//! что кант: заливка поперечной улицы кроет его на перекрёстке, и тротуар
//! обрывается там, где обрывается в жизни. **Разметку** — линии по границам
//! полос ([`lane_count`]: тег `lanes`, иначе дефолт по ширине) — рисует не
//! геометрия, а шейдер поверхностей (`map/surface.rs`) по локальным
//! координатам ленты (`meshing::ATTRIBUTE_RIBBON`): линия сглажена, гаснет при
//! отдалении и **рвётся на перекрёстках** — по общим узлам ways
//! (`roads/junctions.rs`), а не по торцам, так что way, разрезанный посреди
//! квартала, несёт линию сквозь стык, а сквозная улица теряет её ровно на
//! ширину поперечной. Широкие улицы кладутся поверх узких: заливка
//! магистрали кроет торец жилой улицы, и в перекрёстке остаётся разметка
//! магистрали с разрывом под въезд, а не обрубок линии въезда поверх неё.

use std::borrow::Cow;
use std::f32::consts::PI;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::map::footprint::casing_width;
use crate::map::meshing::{
    Break, Markings, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin, merge_close_points,
    miter_offsets,
};
use crate::map::osm::{MapData, RoadClass, RoadLine, WallLine};
use crate::map::surface::{LayerMaterial, SurfaceKind, SurfaceMaterials, spawn_layer};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};
use crate::settings::{
    Z_ALLEY, Z_ALLEY_CASING, Z_BRIDGE, Z_BRIDGE_CASING, Z_BRIDGE_SHADOW, Z_BUILDING, Z_ROAD,
    Z_ROAD_CASING, Z_SIDEWALK,
};

/// Путь тени настила: та же осевая, сдвинутая по свету на высоту моста,
/// **сходящую к нулю у торцов**.
///
/// Без схождения тень вылезала на дорогу в месте примыкания: у береговой
/// опоры настил лежит на земле, и тени там нет вовсе, а сдвинутая на полную
/// высоту лента выезжала за торец моста и ложилась тёмной полосой поперёк
/// подходящей улицы. Подъём считается по длине дуги: `RAMP_SHARE` длины с
/// каждого конца (но не больше `RAMP_MAX`) — это и есть насыпь.
///
/// Осевая для этого **догущается** ([`densify`]): подъём живёт в вершинах, а
/// прямой мост в OSM — это ровно две точки, и обе они торцы. Без догущения
/// `rise` в обеих ноль, тень ложится точь-в-точь под настил и пропадает
/// целиком — так мостики через пруд и не отбрасывали тени вовсе, тогда как
/// изломанный путепровод (вершин много) давал рваную полосу: подъём прыгал от
/// вершины к вершине.
///
/// Подъём вдобавок **зажат остатком пролёта**: если тень едет вдоль моста, то
/// на расстоянии `at` от торца ей позволено уехать не дальше чем на `at`.
/// Гладкая насыпь поднимается быстрее, чем набирается длина, и у короткого
/// моста тень успевала перевалить за торец и лечь тёмным клином на дорогу —
/// тот самый клин, что торчал из-под каждого мостика через канал.
fn bridge_shadow_path(points: &[Vec2]) -> Vec<ShadowPoint> {
    // слипшиеся точки OSM вырождают нормаль стыка — то же, что делает лента
    let merged = merge_close_points(points, false, SHADOW_STEP / 4.0);
    let dense = densify(&merged, SHADOW_STEP);
    if dense.len() < 2 {
        return Vec::new();
    }
    let mut along = Vec::with_capacity(dense.len());
    let mut travelled = 0.0;
    for (index, point) in dense.iter().enumerate() {
        if index > 0 {
            travelled += point.distance(dense[index - 1]);
        }
        along.push(travelled);
    }
    let length = travelled;
    let offset = shadow_dir() * (bridge_height(length) * shadow_length_scale());
    let ramp = (length * RAMP_SHARE).clamp(f32::EPSILON, RAMP_MAX);
    let last = dense.len() - 1;
    (0..dense.len())
        .map(|index| {
            let (point, at) = (dense[index], along[index]);
            let raised = (at.min(length - at) / ramp).clamp(0.0, 1.0);
            // плавно, а не изломом: у настоящей насыпи профиль сглажен
            let rise = raised * raised * (3.0 - 2.0 * raised);
            // ход тени вдоль моста и сколько его осталось до торца впереди
            let tangent = (dense[(index + 1).min(last)] - dense[index.saturating_sub(1)])
                .normalize_or(Vec2::X);
            let travel = offset.dot(tangent);
            let room = if travel > 0.0 { length - at } else { at };
            let rise = if travel.abs() > f32::EPSILON {
                rise.min(room / travel.abs())
            } else {
                rise
            };
            ShadowPoint {
                at: point + offset * rise,
                rise,
            }
        })
        .collect()
}

/// Высота настила над тем, что под ним, м: [`SPAN_TO_HEIGHT`] пролёта, но не
/// выше [`BRIDGE_HEIGHT`].
///
/// Высота росла с пролётом всегда — просто раньше об этом не спрашивали, и
/// всякий way с `bridge=yes` поднимался на шесть метров. В OSM этот тег носят
/// не только пролёты: им же размечены сходы с набережной, тротуар вдоль
/// путепровода, четырёхметровый переход через ливнёвку. Шестиметровая тень от
/// двадцатиметровой дорожки, под которой на месте ничего нет, — самое заметное
/// враньё, какое карта может себе позволить, потому что тень читается как
/// высота.
fn bridge_height(span: f32) -> f32 {
    (span * SPAN_TO_HEIGHT).min(BRIDGE_HEIGHT)
}

/// Точка теневой ленты: куда съехал настил и насколько он в этом месте поднят
/// (0 у береговой опоры, 1 на полной высоте). Подъём нужен и после сдвига —
/// им же сходит на нет кайма ([`push_bridge_shadow`]).
struct ShadowPoint {
    at: Vec2,
    rise: f32,
}

/// Ломаная, догущённая до шага не крупнее `step`: исходные вершины остаются на
/// месте, между ними встают промежуточные. Нужна там, где вдоль ленты меняется
/// не только направление, но и величина — здесь высота настила.
fn densify(points: &[Vec2], step: f32) -> Vec<Vec2> {
    let Some((last, rest)) = points.split_last() else {
        return Vec::new();
    };
    let mut dense = Vec::with_capacity(rest.len() + 1);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        dense.push(from);
        let parts = (from.distance(to) / step).ceil().max(1.0);
        for part in 1..parts as usize {
            dense.push(from.lerp(to, part as f32 / parts));
        }
    }
    dense.push(*last);
    dense
}

/// Доля длины моста, на которой настил поднимается от земли до полной высоты,
/// потолок этой длины в метрах и шаг, которым осевая догущается под подъём.
const RAMP_SHARE: f32 = 0.25;
const RAMP_MAX: f32 = 25.0;
const SHADOW_STEP: f32 = 2.0;

/// Насколько тень настила шире самого настила с каждой стороны, м. Метр — это
/// тень перил, толщина плиты и полоса воды, которой настил закрыл небо; от
/// высоты моста, в отличие от сдвига, эта кайма почти не зависит (перила у
/// пешеходного мостика те же, что у моста через Упу), но вместе с настилом
/// садится на землю у торцов. См. [`push_bridge_shadow`].
const SHADOW_SPREAD: f32 = 1.0;

/// Потолок высоты настила над тем, что под ним, м, и высота на метр пролёта.
/// Длину тени высота даёт тем же котангенсом высоты солнца, что у домов и
/// вагонов: путепровод над улицей поднят метров на шесть, и тень от него —
/// самое заметное, что бывает на воде под мостом. Полную высоту набирает
/// пролёт от 48 м — мост через Упу (137 м) и развязка (571 м) на потолке,
/// мостик через пруд (14 м) поднят на метр восемьдесят.
const BRIDGE_HEIGHT: f32 = 6.0;
const SPAN_TO_HEIGHT: f32 = 1.0 / 8.0;

/// Проезжая часть — асфальт: серый, заметно темнее тротуара и земли, как на
/// детальных картах 2ГИС и Яндекса. Белой (osm-carto) она была, пока не
/// появилась разметка: белую линию на белом не видно, а на сером сетка улиц
/// вдобавок перестаёт сливаться с дворами.
/// Открыт наружу витрине машин: ряд обязан стоять на том же асфальте, что в
/// городе, — на своём сером ступень яркости между кузовом и покрытием была бы
/// не та.
pub const ROAD_COLOR: Color = Color::srgb(0.655, 0.66, 0.675);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Тротуар — светлый бетон между асфальтом и тёплой землёй: светлее проезжей
/// части на четверть, и именно эта ступень яркости читается как бордюр.
const SIDEWALK_COLOR: Color = Color::srgb(0.82, 0.815, 0.80);
/// Доля ширины улицы на тротуар с каждой стороны и её пределы, м: у
/// магистрали в 16 м тротуар в 3 м, у жилой улицы в 8 м — 1.8 м.
const SIDEWALK_SHARE: f32 = 0.22;
const SIDEWALK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 1.2..=3.0;
/// Улицы у́же этого — проезды (`service`, 5 м): ни тротуара, ни разметки, ни
/// разрыва в разметке улицы, к которой проезд примыкает.
const STREET_MIN_WIDTH: f32 = 8.0;

/// Полоса не у́же этого, м: `lanes=6` на десятиметровой ленте — данные о
/// настоящей улице, а лента у нас по классу, и лишние полосы отбрасываются.
const MIN_LANE_WIDTH: f32 = 2.5;
/// Полос по умолчанию, когда тега `lanes` нет: двусторонней улице — по паре
/// на каждые 7 м ширины (8 и 10 м — две полосы, 12 и 16 — четыре),
/// односторонней — по полосе на 4.5 м (8 м — одна, без линий; 16 — три).
const TWOWAY_METERS_PER_LANE_PAIR: f32 = 7.0;
const ONEWAY_METERS_PER_LANE: f32 = 4.5;

/// Кант дороги — затемнённая заливка, как у osm-carto (улица в тёмном канте);
/// темнее асфальта. Отдельным слоем под заливкой: заливки всех дорог кроют
/// канты всех дорог, поэтому кант никогда не режет перекрёсток пополам.
const ROAD_CASING_COLOR: Color = Color::srgb(0.45, 0.45, 0.46);
const ALLEY_CASING_COLOR: Color = Color::srgb(0.729, 0.678, 0.549);

/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

/// Бордюр моста — светлый бетонный парапет над серым настилом, общий для
/// улиц и пешеходных мостиков. Толщины (и почему их диапазоны не
/// пересекаются) — в `map::footprint`.
const BRIDGE_CURB_COLOR: Color = Color::srgb(0.80, 0.80, 0.79);

/// Изломы мельче Chaikin не срезает: прямые участки обязаны остаться точками
/// OSM, иначе сглаживание съедает и без того редкую геометрию длинных улиц.
const MIN_SMOOTH_ANGLE: f32 = 10.0 * PI / 180.0;
/// Доля сегмента, отрезаемая с каждой стороны излома (классический Chaikin).
const CHAIKIN_CUT: f32 = 0.25;

/// Чем закрыт излом ленты дороги.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadJoin {
    /// Статус-кво до сглаживания: свой квад на сегмент, оба конца продлены на
    /// полуширины. Оставлен, чтобы можно было сравнить с прежней картинкой.
    Square,
    /// Сведение по биссектрисе с ограничением длины стыка.
    Miter,
    /// Дуга на внешней стороне излома + полудиск на торце — вид osm-carto.
    #[default]
    Round,
}

impl RoadJoin {
    pub const ALL: [Self; 3] = [Self::Square, Self::Miter, Self::Round];

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Miter => "Miter",
            Self::Round => "Round",
        }
    }

    /// Излом и торец ленты `MeshBuilder`. `None` — `Square`: ленты у него нет
    /// вовсе, это `push_polyline` с продлёнными торцами. Одна таблица на всех,
    /// кто кладёт ленту дороги ([`push_ribbon`], [`push_street_fill`]) —
    /// разойдясь, они дали бы двум слоям одной улицы разные торцы.
    fn ribbon_shape(self) -> Option<(RibbonJoin, RibbonCap)> {
        match self {
            Self::Square => None,
            Self::Miter => Some((RibbonJoin::Miter, RibbonCap::Butt)),
            Self::Round => Some((RibbonJoin::Round, RibbonCap::Round)),
        }
    }
}

/// Сколько раз осевая прогоняется через Chaikin перед построением ленты.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadSmoothing {
    /// Осевая ровно по данным OSM — как в самом OSM, где углы остаются острыми.
    Off,
    /// Один проход: улица на повороте перестаёт ломаться под углом, а рисунок
    /// сети ещё держится там, где OSM ставил узлы.
    #[default]
    Light,
    Strong,
}

impl RoadSmoothing {
    pub const ALL: [Self; 3] = [Self::Off, Self::Light, Self::Strong];

    fn iterations(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Light => 1,
            Self::Strong => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Light => "Light",
            Self::Strong => "Strong",
        }
    }
}

/// Стиль дорожных лент; переключается панелью Roads и BRP, сохраняется в
/// настройках между запусками. Правка пересобирает дорожные слои
/// ([`rebuild_roads`]).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roads")]
pub struct RoadStyle {
    pub join: RoadJoin,
    pub smoothing: RoadSmoothing,
    /// Тёмный кант по краю дороги отдельным слоем под заливкой.
    pub casing: bool,
    /// Серая полоса тротуара вдоль улиц (не проездов) отдельным слоем под
    /// всеми лентами.
    pub sidewalks: bool,
    /// Разметка полос на проезжей части улиц — линия на каждой границе полос,
    /// с разрывами на перекрёстках; рисует шейдер поверхностей.
    pub markings: bool,
}

impl Default for RoadStyle {
    fn default() -> Self {
        Self {
            join: RoadJoin::default(),
            smoothing: RoadSmoothing::default(),
            casing: false,
            sidewalks: true,
            markings: true,
        }
    }
}

/// Ширина тротуара с одной стороны улицы, м; проезд тротуара не получает.
pub fn sidewalk_width(road_width: f32) -> Option<f32> {
    (road_width >= STREET_MIN_WIDTH).then(|| {
        (road_width * SIDEWALK_SHARE)
            .clamp(*SIDEWALK_WIDTH_RANGE.start(), *SIDEWALK_WIDTH_RANGE.end())
    })
}

/// Проезжая часть улицы — то, что несёт тротуар и разметку и участвует в
/// перекрёстках: класс `Street`, не арка (`passage` идёт сквозь дом), не у́же
/// [`STREET_MIN_WIDTH`]. Мост — тоже: улица через реку не теряет полос.
///
/// Открыт наружу для [`map::cars`](crate::map::cars): «улица, вдоль которой
/// паркуются» — то же самое понятие, что «улица, у которой есть тротуар и
/// разметка», и второй копии предиката у слоя машин быть не должно.
pub fn is_carriageway(road: &RoadLine) -> bool {
    road.class == RoadClass::Street && !road.passage && road.width >= STREET_MIN_WIDTH
}

/// Число полос проезжей части: тег `lanes`, иначе дефолт по ширине, и не
/// больше, чем влезает по [`MIN_LANE_WIDTH`]. Кольцо — всегда одна полоса:
/// на однополосном кольце линий нет, а рвать линию двухполосного на каждом
/// въезде хуже, чем не рисовать её вовсе.
pub fn lane_count(road: &RoadLine) -> u8 {
    if road.roundabout {
        return 1;
    }
    let most = ((road.width / MIN_LANE_WIDTH).floor() as u8).max(1);
    let lanes = match road.lanes {
        Some(lanes) => lanes,
        None if road.oneway => (road.width / ONEWAY_METERS_PER_LANE).floor() as u8,
        None => 2 * (road.width / TWOWAY_METERS_PER_LANE_PAIR).round() as u8,
    };
    lanes.clamp(1, most)
}

/// Разметка проезжей части: линии лежат на границах полос, так что
/// однополосной рисовать нечего.
fn road_markings(road: &RoadLine) -> Option<Markings> {
    if !is_carriageway(road) {
        return None;
    }
    let lanes = lane_count(road);
    (lanes >= 2).then_some(Markings {
        lanes,
        oneway: road.oneway,
    })
}

/// Дорожный слой карты — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
pub struct RoadLayerTag;

/// Спавн дорожных слоёв в выбранном стиле. Вызывается из `spawn_map` при входе
/// в мир и из [`rebuild_roads`] при переключении стиля.
pub fn spawn_roads(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    surfaces: &SurfaceMaterials,
    style: RoadStyle,
    map: &MapData,
) {
    let started = std::time::Instant::now();
    let (roads, walls): (&[RoadLine], &[WallLine]) = (&map.roads, &map.walls);
    // вершинные цвета — плоский материал один, белый; фактурные — по виду
    // поверхности, из `SurfaceMaterials`
    let flat = materials.add(Color::WHITE);
    // тень моста полупрозрачна, поэтому у неё свой материал с блендингом:
    // белый непрозрачный съел бы альфу вершинного цвета
    let shadow = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    // перекрёстки нужны только разметке: без неё и рвать нечего
    let junctions = style
        .markings
        .then(|| junctions::marking_breaks(roads, is_carriageway));
    // широкие улицы поверх узких — см. доку модуля
    let mut order: Vec<usize> = (0..roads.len()).collect();
    order.sort_by(|&a, &b| roads[a].width.total_cmp(&roads[b].width));

    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut alley_casings = MeshBuilder::default();
    let mut alleys = MeshBuilder::with_surface_coords();
    let mut street_casings = MeshBuilder::default();
    let mut streets = MeshBuilder::with_surface_coords();
    // Настилы мостов — один меш на улицы и пешеходные мостики разом: белая и
    // песочная заливки соседствуют, и порядок перекрытия моста над мостом —
    // порядок пуша. Мост над мостом — редкость, четыре слоя ради него не нужны.
    let mut bridge_casings = MeshBuilder::default();
    let mut bridge_fills = MeshBuilder::with_surface_coords();
    // тень моста — на то, над чем он проходит: воду, дорогу, пути
    let mut bridge_shadows = MeshBuilder::default();
    let mut wall_ribbons = MeshBuilder::default();

    for index in order {
        let road = &roads[index];
        let (casing_color, color) = match road.class {
            RoadClass::Street => (ROAD_CASING_COLOR, ROAD_COLOR),
            RoadClass::Alley => (ALLEY_CASING_COLOR, ALLEY_COLOR),
        };
        let points = centerline(road, style.smoothing);
        // разметка и её разрывы — только пока она включена
        let (markings, breaks) = match &junctions {
            Some(found) => (road_markings(road), found.breaks[index].as_slice()),
            None => (None, &[][..]),
        };
        if road.bridge {
            // бордюр — всегда, независимо от style.casing: он и есть мост
            push_bridge_curb(
                &mut bridge_casings,
                &points,
                2.0 * road.curb_reach(),
                style.join,
            );
            // Тень настила — тот же настил, сдвинутый по свету на высоту
            // моста. Ни один другой слой её не даёт: наземные тени считают
            // только дома, а мост через Упу — самая заметная вещь на воде.
            push_bridge_shadow(
                &mut bridge_shadows,
                &bridge_shadow_path(&points),
                road.curb_reach(),
            );
            bridge_fills.set_markings(markings);
            push_street_fill(
                &mut bridge_fills,
                &points,
                road.width,
                color.to_linear(),
                style.join,
                breaks,
            );
            continue;
        }
        let (casing, fill) = match road.class {
            RoadClass::Street => (&mut street_casings, &mut streets),
            RoadClass::Alley => (&mut alley_casings, &mut alleys),
        };
        if style.sidewalks
            && is_carriageway(road)
            && let Some(sidewalk) = sidewalk_width(road.width)
        {
            push_ribbon(
                &mut sidewalks,
                &points,
                road.width + 2.0 * sidewalk,
                SIDEWALK_COLOR.to_linear(),
                style.join,
            );
        }
        if style.casing {
            let width = road.width + 2.0 * casing_width(road.width);
            push_ribbon(casing, &points, width, casing_color.to_linear(), style.join);
        }
        fill.set_markings(markings);
        push_street_fill(
            fill,
            &points,
            road.width,
            color.to_linear(),
            style.join,
            breaks,
        );
    }

    for wall in walls {
        push_ribbon(
            &mut wall_ribbons,
            &wall.points,
            wall.width,
            WALL_COLOR.to_linear(),
            style.join,
        );
    }

    let vertices = [
        &sidewalks,
        &alley_casings,
        &alleys,
        &street_casings,
        &streets,
        &bridge_casings,
        &bridge_fills,
        &wall_ribbons,
    ]
    .iter()
    .map(|builder| builder.vertex_count())
    .sum::<usize>();

    let surface = |kind| LayerMaterial::Surface(surfaces.handle(kind));
    for (builder, z, name, material) in [
        (
            sidewalks,
            Z_SIDEWALK,
            "sidewalks",
            surface(SurfaceKind::Sidewalk),
        ),
        (
            alley_casings,
            Z_ALLEY_CASING,
            "alley_casings",
            LayerMaterial::Flat(flat.clone()),
        ),
        (alleys, Z_ALLEY, "alleys", surface(SurfaceKind::Alley)),
        (
            street_casings,
            Z_ROAD_CASING,
            "road_casings",
            LayerMaterial::Flat(flat.clone()),
        ),
        (streets, Z_ROAD, "roads", surface(SurfaceKind::Street)),
        (
            bridge_shadows,
            Z_BRIDGE_SHADOW,
            "bridge_shadows",
            LayerMaterial::Flat(shadow.clone()),
        ),
        (
            bridge_casings,
            Z_BRIDGE_CASING,
            "bridge_casings",
            LayerMaterial::Flat(flat.clone()),
        ),
        (
            bridge_fills,
            Z_BRIDGE,
            "bridges",
            surface(SurfaceKind::Street),
        ),
        (
            wall_ribbons,
            Z_WALL,
            "walls",
            LayerMaterial::Flat(flat.clone()),
        ),
    ] {
        spawn_layer(commands, meshes, builder, z, name, material, RoadLayerTag);
    }

    info!(
        "road meshing: {vertices} verts in {:?} ({:?}, smoothing {:?}, casing {}, sidewalks {}, markings {}, junctions {})",
        started.elapsed(),
        style.join,
        style.smoothing,
        style.casing,
        style.sidewalks,
        style.markings,
        junctions.as_ref().map_or(0, |found| found.junctions),
    );
}

/// Пересборка дорожных слоёв после переключения стиля из UI или BRP: деспавн
/// старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    surfaces: Res<SurfaceMaterials>,
    style: Res<RoadStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<RoadLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        &surfaces,
        *style,
        &map,
    );
}

/// Тень настила — лента переменной ширины по [`bridge_shadow_path`], **шире
/// самого настила** на [`SHADOW_SPREAD`] с каждой стороны, и торцы у неё
/// прямые.
///
/// Уширение — не украшение, а единственное, что даёт мосту тень **вдоль
/// света**. Тень плиты это её силуэт, сдвинутый по солнцу; у ленты сдвиг
/// раскладывается на поперечную часть (видимая полоса сбоку) и продольную
/// (лента съезжает сама по себе и остаётся под настилом). Мостики через пруд
/// на Упе идут ровно по азимуту солнца — поперечной части у них нет, и честная
/// тень-сдвиг у них невидима до последнего пикселя. На снимке они, однако,
/// обведены тёмным: вода под настилом не освещена небом, к этому добавляется
/// тень перил и толщина самой плиты. Это и есть `SHADOW_SPREAD` — кайма,
/// сходящая к нулю там же, где садится на землю настил.
///
/// Торцы прямые (лента режется по вершинам, полудиска нет ни на одном конце):
/// круглый торец и был тем артефактом, из-за которого тень «оставалась у
/// дороги» — полудиск радиусом в полширины настила вылезал за прямой срез
/// моста и ложился на улицу, к которой мост примыкает.
fn push_bridge_shadow(builder: &mut MeshBuilder, path: &[ShadowPoint], reach: f32) {
    if path.len() < 2 {
        return;
    }
    let color = SHADOW_COLOR.to_linear();
    let centers: Vec<Vec2> = path.iter().map(|point| point.at).collect();
    // единичные нормали стыка: длину каждой задаёт своя полуширина
    let normals = miter_offsets(&centers, false, 1.0);
    let rails: Vec<[Vec2; 2]> = path
        .iter()
        .zip(&normals)
        .map(|(point, normal)| {
            let half = reach + SHADOW_SPREAD * point.rise;
            [point.at + *normal * half, point.at - *normal * half]
        })
        .collect();
    // по четырёхугольнику на сегмент: соседние делят ребро вершина в вершину,
    // так что полупрозрачная лента нигде не накладывается сама на себя
    for pair in rails.windows(2) {
        let ([left, right], [next_left, next_right]) = (pair[0], pair[1]);
        builder.push_polygon(&[left, next_left, next_right, right], &[], color);
    }
}

/// Бордюр моста: торцы всегда [`RibbonCap::Butt`] — настил кончается ровным
/// срезом, как на 2ГИС. Полудиск `Round` или продление `push_polyline` при
/// `Square` торчали бы бордюрным языком за конец моста, поэтому мимо
/// [`push_ribbon`]-обёртки.
fn push_bridge_curb(builder: &mut MeshBuilder, points: &[Vec2], width: f32, join: RoadJoin) {
    // `Square` — это `push_polyline` с продлёнными торцами, а торцы здесь
    // решены выше; его излом сводится к `Miter`, как у любой метки, а не дороги
    let join = match join {
        RoadJoin::Square | RoadJoin::Miter => RibbonJoin::Miter,
        RoadJoin::Round => RibbonJoin::Round,
    };
    builder.push_ribbon(
        points,
        false,
        width,
        BRIDGE_CURB_COLOR.to_linear(),
        join,
        RibbonCap::Butt,
    );
}

/// Лента выбранного стиля. Общая с подложкой аллей (`map::spawn`): у неё те же
/// три настройки, что у дорог, и мапиться на `MeshBuilder` они обязаны одинаково.
pub fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    builder.push_ribbon(points, false, width, color, join, cap);
}

/// Заливка проезжей части — лента с разрывами разметки по перекрёсткам. При
/// `Square` разрывы деть некуда: `push_polyline` знает только торцы, а режим
/// оставлен ради сравнения картинок, не ради разметки.
fn push_street_fill(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
    breaks: &[Break],
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    builder.push_ribbon_broken(
        points,
        width,
        color,
        join,
        [cap; 2],
        RibbonBreaks::At(breaks),
    );
}

/// Осевая, по которой строится лента. Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
fn centerline(road: &RoadLine, smoothing: RoadSmoothing) -> Cow<'_, [Vec2]> {
    if road.passage {
        return Cow::Borrowed(&road.points);
    }
    smooth_path(&road.points, road.width, smoothing)
}

/// Сглаживание осевой на копии — общее для дорог, рельсов и зелёной полосы под
/// аллеей (`map::spawn`). Длина среза зажата шириной ленты, поэтому ширина
/// здесь параметр, а не константа.
pub fn smooth_path(points: &[Vec2], width: f32, smoothing: RoadSmoothing) -> Cow<'_, [Vec2]> {
    let iterations = smoothing.iterations();
    if iterations == 0 || points.len() < 3 {
        return Cow::Borrowed(points);
    }
    let mut path = points.to_vec();
    for _ in 0..iterations {
        path = chaikin(&path, width);
    }
    Cow::Owned(path)
}

/// Срезание углов по Chaikin: излом заменяется парой точек на прилежащих
/// сегментах. Срезаются только изломы круче [`MIN_SMOOTH_ANGLE`], а длина
/// среза зажата шириной дороги — иначе на длинных сегментах осевая уезжает от
/// данных OSM на десятки метров и дорога перестаёт совпадать с домами.
/// Концы пути закреплены.
fn chaikin(points: &[Vec2], width: f32) -> Vec<Vec2> {
    let mut path = Vec::with_capacity(points.len() * 2);
    path.push(points[0]);
    for index in 1..points.len() - 1 {
        let (previous, corner, next) = (points[index - 1], points[index], points[index + 1]);
        let (Some(incoming), Some(outgoing)) = (
            (corner - previous).try_normalize(),
            (next - corner).try_normalize(),
        ) else {
            path.push(corner);
            continue;
        };
        if incoming.angle_to(outgoing).abs() < MIN_SMOOTH_ANGLE {
            path.push(corner);
            continue;
        }
        let back = (corner.distance(previous) * CHAIKIN_CUT).min(width);
        let forward = (next.distance(corner) * CHAIKIN_CUT).min(width);
        path.push(corner - incoming * back);
        path.push(corner + outgoing * forward);
    }
    path.push(points[points.len() - 1]);
    path
}

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин обязан
/// рваться на тех же перекрёстках, на которых рвётся разметка, и второго
/// восстановления узлов по общим нодам заводить незачем.
pub(super) mod junctions;

#[cfg(test)]
mod tests;
