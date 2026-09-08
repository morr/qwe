//! Скатные крыши малых домов. `roof:shape` в OSM стоит у 283 зданий Тулы из
//! 7465, так что форма крыши не читается из тегов, а **выводится**: частный
//! дом (`BuildingUse::House`) и любая мелкая коробка без назначения получают
//! скатную крышу, остальное — плоскую. Плоская крыша у частного сектора была
//! главной причиной, по которой окраины читались как склад контейнеров.
//!
//! Скатных две, и выбор между ними — форма контура плюс посев дома
//! ([`roofing`]):
//!
//! * **двускатная** ([`gable_roof`]) — конёк вдоль длинной оси минимального
//!   описанного прямоугольника (OBB); крыша рисуется по этому прямоугольнику,
//!   а не по контуру, — у настоящего дома скаты и так нависают над стеной.
//!   Поэтому она ставится только на контур, заполняющий прямоугольник почти
//!   целиком ([`RECT_FILL_MIN`]): иначе из дома торчала бы крыша буквой Г.
//! * **вальмовая** ([`HipRoof`]) — скаты по всему контуру и площадка конька
//!   внутри, построенные вдвигом контура на miter-офсетах. Ей форма контура
//!   безразлична, и именно она достаётся Г-образным домам, которые до сих пор
//!   оставались плоскими среди скатных соседей.

use bevy::prelude::*;

use super::shade_by_light;
use crate::map::meshing::{merge_close_points, miter_offsets};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};

/// Какую долю своего описанного прямоугольника контур обязан заполнять,
/// чтобы прямоугольная крыша не торчала из него. Дом с эркером или срезанным
/// углом проходит, Г-образный (≈0.5–0.7) — нет.
const RECT_FILL_MIN: f32 = 0.85;
/// Здание без назначения не крупнее этого, м², считается частным домом:
/// в Туле `building=yes` стоит на 4004 контурах из 7465, и за окраины
/// отвечает именно эта половина.
const SMALL_FOOTPRINT_MAX: f32 = 250.0;
/// Подъём конька на метр половины ширины дома — тангенс угла ската
/// (0.8 ≈ 39°). Круче, чем у типовой шиферной крыши: на карте сверху рисуется
/// доля высоты, и пологий скат вовсе не читался бы.
const ROOF_PITCH: f32 = 0.8;
/// Настоящих метров конька над карнизом, не больше: широкий ангар с
/// пятиметровым коньком ещё дом, с десятиметровым — цирк.
const ROOF_RISE_MAX: f32 = 5.0;
/// Насколько скат, повёрнутый к свету, светлее базового тона крыши, а
/// отвёрнутый — темнее. Мягче стен (`WALL_*_MIX`): крыша смотрит в небо и
/// освещена вся, разница только в наклоне. Значения пересчитаны из прежнего
/// смешивания в линейном пространстве, чтобы видимый шаг остался тем же.
const SLOPE_LIT_MIX: f32 = 0.14;
const SLOPE_SHADED_MIX: f32 = 0.11;

/// Двускатная крыша, разложенная на куски для painter's algorithm:
/// фронтоны рисуются со стенами, скаты — поверх.
pub(super) struct GableRoof {
    /// Два ската: четырёхугольник (карниз, карниз, конёк, конёк) и тон.
    pub(super) slopes: [([Vec2; 4], LinearRgba); 2],
    /// Два фронтона: торец прямоугольника `(a, b)` на уровне карниза (CCW,
    /// наружная нормаль — правый перпендикуляр) и вершина конька над ним.
    pub(super) gables: [((Vec2, Vec2), Vec2); 2],
}

/// Вальмовая крыша: скаты по **всему** контуру и площадка конька внутри.
///
/// Строится не straight skeleton'ом, а вдвигом контура внутрь на
/// [`HIP_INSET`] теми же miter-офсетами, что дают дальний край каймы: скат —
/// квад между ребром контура и его сдвинутой парой, конёк — то, что осталось
/// внутри. Для выпуклого дома это и есть вальма; для Г-образного — вальма с
/// плоской верхушкой, то есть ровно то, что видно на снимке, и то, чего
/// straight skeleton стоил бы на порядок дороже.
pub(super) struct HipRoof {
    /// Скаты: четырёхугольник (карниз, карниз, конёк, конёк) и тон, по одному
    /// на ребро контура.
    pub(super) slopes: Vec<([Vec2; 4], LinearRgba)>,
    /// Площадка конька — вдвинутый контур и его тон.
    pub(super) ridge: (Vec<Vec2>, LinearRgba),
}

/// Что за крыша у дома. Плоская — не «крыши нет», а именно плоская кровля со
/// своим материалом и парапетом.
pub(super) enum Roofing {
    Gable(GableRoof),
    Hip(HipRoof),
    Flat,
}

/// Вылет ската вальмы по плану, м, и потолок этого вылета в долях толщины
/// контура (`площадь / периметр`): у узкого дома скаты обязаны сойтись, а не
/// вывернуться наизнанку — тот же зажим, что у каймы.
const HIP_INSET: f32 = 2.2;
const HIP_INSET_SHARE: f32 = 0.38;
/// Насколько площадка конька светлее базового тона: она смотрит прямо в небо,
/// а скаты — вбок.
const RIDGE_LIGHTEN: f32 = 0.06;

/// Крыша дома целиком: двускатная, вальмовая или плоская.
///
/// Двускатную получает почти прямоугольный дом (её конёк идёт по длинной оси
/// и требует прямоугольника), вальмовую — тот же дом, если так выпал посев,
/// **и** всякий негодный для двускатной контур: Г-образный дом до сих пор
/// оставался с плоской крышей среди скатных соседей, что на снимке частного
/// сектора видно сразу.
pub(super) fn roofing(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    seed: u32,
) -> Roofing {
    if !is_gabled(building) {
        return Roofing::Flat;
    }
    let hipped = (seed >> 5) % 10 < HIPPED_SHARE;
    if !hipped && let Some(roof) = gable_roof(building, lift, &ridge_lift, base) {
        return Roofing::Gable(roof);
    }
    match hip_roof(building, lift, &ridge_lift, base) {
        Some(roof) => Roofing::Hip(roof),
        // на совсем узком контуре вальма выворачивается, а двускатная не
        // встала — пусть будет плоской, это по крайней мере не врёт
        None => gable_roof(building, lift, &ridge_lift, base).map_or(Roofing::Flat, Roofing::Gable),
    }
}

/// Сколько домов из десяти кроются вальмой, а не двускатной. В частном
/// секторе двускатных всё же больше, но вальма — не редкость.
const HIPPED_SHARE: u32 = 4;

/// Вальмовая крыша над контуром, поднятым на `lift`. `None` — контур слишком
/// тонкий, чтобы скаты сошлись.
fn hip_roof(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<HipRoof> {
    let ring = merge_close_points(&building.outer, true, HIP_INSET / 4.0);
    if ring.len() < 3 {
        return None;
    }
    let area = signed_ring_area(&ring);
    let inset = HIP_INSET.min(HIP_INSET_SHARE * area.abs() / perimeter(&ring));
    if inset < MIN_HIP_INSET {
        return None;
    }
    // офсеты смотрят влево по ходу обхода: у CCW-кольца это внутрь
    let side = if area > 0.0 { 1.0 } else { -1.0 };
    let offsets = miter_offsets(&ring, true, inset);
    let rise = ridge_lift((inset * ROOF_PITCH).min(ROOF_RISE_MAX));
    let inner: Vec<Vec2> = ring
        .iter()
        .zip(&offsets)
        .map(|(point, offset)| *point + *offset * side + lift + rise)
        .collect();

    let mut slopes = Vec::with_capacity(ring.len());
    for index in 0..ring.len() {
        let next = (index + 1) % ring.len();
        let (a, b) = (ring[index] + lift, ring[next] + lift);
        let outward = Vec2::new((b - a).y, -(b - a).x).normalize_or_zero() * side;
        let tone = shade_by_light(base, outward, SLOPE_LIT_MIX, SLOPE_SHADED_MIX);
        slopes.push(([a, b, inner[next], inner[index]], tone.into()));
    }
    Some(HipRoof {
        slopes,
        ridge: (inner, base.mix(&Srgba::WHITE, RIDGE_LIGHTEN).into()),
    })
}

/// Тоньше этого вдвиг не имеет смысла: скат в двадцать сантиметров не виден,
/// а вершин на контур столько же.
const MIN_HIP_INSET: f32 = 0.4;

/// Периметр замкнутого контура.
fn perimeter(ring: &[Vec2]) -> f32 {
    (0..ring.len())
        .map(|index| ring[index].distance(ring[(index + 1) % ring.len()]))
        .sum()
}

/// Крыша этого дома — скатная (двускатная или вальмовая)?
pub(super) fn is_gabled(building: &PolyArea) -> bool {
    // Кремль вне стилизации по назначению, как и в `base_colors`
    if building.kind == AreaKind::Kremlin {
        return false;
    }
    if !building.holes.is_empty() {
        return false;
    }
    match building.building_use {
        BuildingUse::House => true,
        BuildingUse::Other => signed_ring_area(&building.outer).abs() <= SMALL_FOOTPRINT_MAX,
        _ => false,
    }
}

/// Настоящих метров конька над карнизом для дома шириной `width`.
pub(super) fn ridge_rise(width: f32) -> f32 {
    (width / 2.0 * ROOF_PITCH).min(ROOF_RISE_MAX)
}

/// Двускатная крыша над контуром, поднятым на `lift`; `ridge_lift` — на
/// сколько выше карниза нарисован конёк (в плоских режимах — ноль, и скаты
/// отличаются только тоном). `None` — крыша остаётся плоской: дом не из
/// тех, что [`is_gabled`], или контур не прямоугольный.
pub(super) fn gable_roof(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    if !is_gabled(building) {
        return None;
    }
    let rect = min_area_rect(&building.outer)?;
    let rect_area = (rect[1] - rect[0]).length() * (rect[2] - rect[1]).length();
    if rect_area <= 0.0 || signed_ring_area(&building.outer).abs() / rect_area < RECT_FILL_MIN {
        return None;
    }
    let [c0, c1, c2, c3] = rect.map(|corner| corner + lift);
    let width = (c2 - c1).length();
    let ridge = ridge_lift(ridge_rise(width));
    let (r0, r1) = ((c0 + c3) / 2.0 + ridge, (c1 + c2) / 2.0 + ridge);

    // скат c0–c1 смотрит наружу правым перпендикуляром к c0→c1 (CCW-обход),
    // противоположный — ровно наоборот
    let long = (c1 - c0).normalize_or_zero();
    let outward = Vec2::new(long.y, -long.x);
    Some(GableRoof {
        slopes: [
            (
                [c0, c1, r1, r0],
                shade_by_light(base, outward, SLOPE_LIT_MIX, SLOPE_SHADED_MIX).into(),
            ),
            (
                [r0, r1, c2, c3],
                shade_by_light(base, -outward, SLOPE_LIT_MIX, SLOPE_SHADED_MIX).into(),
            ),
        ],
        gables: [((c1, c2), r1), ((c3, c0), r0)],
    })
}

/// Минимальный по площади описанный прямоугольник кольца, CCW, первое ребро
/// вдоль длинной оси. У такого прямоугольника одна сторона лежит на ребре
/// выпуклой оболочки, а рёбра оболочки — подмножество рёбер контура, так что
/// перебор направлений всех рёбер находит оптимум без построения оболочки:
/// контуров тысячи, вершин в каждом — единицы.
pub(super) fn min_area_rect(ring: &[Vec2]) -> Option<[Vec2; 4]> {
    if ring.len() < 3 {
        return None;
    }
    let mut best: Option<(f32, Vec2, Vec2, Vec2)> = None;
    for i in 0..ring.len() {
        let Some(u) = (ring[(i + 1) % ring.len()] - ring[i]).try_normalize() else {
            continue;
        };
        let v = Vec2::new(-u.y, u.x);
        let (mut u_min, mut u_max, mut v_min, mut v_max) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for point in ring {
            let (pu, pv) = (point.dot(u), point.dot(v));
            u_min = u_min.min(pu);
            u_max = u_max.max(pu);
            v_min = v_min.min(pv);
            v_max = v_max.max(pv);
        }
        let area = (u_max - u_min) * (v_max - v_min);
        if best.is_none_or(|(best_area, ..)| area < best_area) {
            best = Some((area, u, Vec2::new(u_min, v_min), Vec2::new(u_max, v_max)));
        }
    }
    let (_, u, low, high) = best?;
    let v = Vec2::new(-u.y, u.x);
    let corner = |pu: f32, pv: f32| u * pu + v * pv;
    let mut rect = [
        corner(low.x, low.y),
        corner(high.x, low.y),
        corner(high.x, high.y),
        corner(low.x, high.y),
    ];
    // длинная ось первой: у CCW-квадрата сдвиг на одну вершину сохраняет обход
    if high.x - low.x < high.y - low.y {
        rect.rotate_left(1);
    }
    Some(rect)
}
