//! Двускатное семейство скатных крыш: обычная двускатная, полувальмовая,
//! ломаная мансардная, односкатная и крестовая над Г-, Т- и П-образным домом.
//! Форму прямоугольному дому выбирает посев ([`house_roof`]) и собирает
//! [`house_gable`], Г-образному — [`cross_gable`] по разрезу контура
//! ([`rect_splits`]); факты формы, вальма и шатёр остаются в родителе.

use bevy::prelude::*;

use super::{
    DormerFace, GABLE_OVERHANG_MAX, GableForm, GableRoof, HouseFootprint, RECT_FILL_MIN,
    ROOF_PITCH, ROOF_RISE_MAX, SHED_FOOTPRINT_MAX, bounding_rect, overhang, pitched_rise,
    rect_area, ridge_rise, shape_seed, slope_tone,
};
use crate::map::buildings::classify::BuildingClass;
use crate::map::meshing::{merge_close_points, min_area_rect};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{BuildingUse, PolyArea};

/// Прямоугольник, над которым встаёт двускатная крыша; `None` — не встаёт:
/// контур заполняет его хуже [`RECT_FILL_MIN`] или какой-то его угол висит
/// дальше [`GABLE_OVERHANG_MAX`] от стены. Крыша рисуется по прямоугольнику,
/// стены — по контуру, и на пустом углу из-под крыши не выходит ни одна стена.
pub(super) fn gable_rect(building: &PolyArea, fp: Option<HouseFootprint>) -> Option<[Vec2; 4]> {
    let (rect, fill) = bounding_rect(building, fp)?;
    (fill >= RECT_FILL_MIN && overhang(&building.outer, &rect) <= GABLE_OVERHANG_MAX)
        .then_some(rect)
}

/// Двускатная крыша над контуром, поднятым на `lift`; `ridge_lift` — на
/// сколько выше карниза нарисован конёк (в плоских режимах — ноль, и скаты
/// отличаются только тоном). `None` — крыша остаётся плоской: дом не из
/// тех, что [`is_pitched`], или контур не прямоугольный.
pub(in crate::map::buildings) fn gable_roof(
    building: &PolyArea,
    class: &BuildingClass,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    fp: Option<HouseFootprint>,
) -> Option<GableRoof> {
    if !class.is_pitched() {
        return None;
    }
    gable_over(
        building,
        lift,
        ridge_lift,
        base,
        ROOF_PITCH,
        ROOF_RISE_MAX,
        fp,
    )
}

/// Двускатная крыша заданной крутизны — без вопроса, положена ли она дому:
/// жилому дому это решает [`is_pitched`], храму — его вера.
#[allow(clippy::too_many_arguments)]
pub(super) fn gable_over(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    pitch: f32,
    rise_max: f32,
    fp: Option<HouseFootprint>,
) -> Option<GableRoof> {
    let rect = gable_rect(building, fp)?;
    Some(plain_gable(
        &Frame::of(rect, lift),
        &ridge_lift,
        base,
        pitched_rise((rect[2] - rect[1]).length(), pitch, rise_max),
    ))
}

/// Прямоугольник крыши, уже поднятый на карниз: углы CCW, `c0→c1` — вдоль
/// конька.
struct Frame {
    c: [Vec2; 4],
    /// Единичный вектор вдоль конька.
    long: Vec2,
    length: f32,
    width: f32,
}

impl Frame {
    fn of(rect: [Vec2; 4], lift: Vec2) -> Self {
        let c = rect.map(|corner| corner + lift);
        Self {
            c,
            long: (c[1] - c[0]).normalize_or_zero(),
            length: (c[1] - c[0]).length(),
            width: (c[2] - c[1]).length(),
        }
    }

    /// Наружная нормаль ската у карниза `c0–c1` — правый перпендикуляр к
    /// `c0→c1`; противоположный скат смотрит ровно наоборот.
    fn outward(&self) -> Vec2 {
        Vec2::new(self.long.y, -self.long.x)
    }

    /// Середины торцов на уровне карниза: у `c0–c3` и у `c1–c2`.
    fn mids(&self) -> (Vec2, Vec2) {
        let [c0, c1, c2, c3] = self.c;
        ((c0 + c3) / 2.0, (c1 + c2) / 2.0)
    }
}

/// Обычная двускатная с подъёмом конька `rise` настоящих метров.
fn plain_gable(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let ridge = ridge_lift(rise);
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let outward = frame.outward();
    GableRoof {
        form: GableForm::Gable,
        slopes: vec![
            (vec![c0, c1, r1, r0], slope_tone(base, outward, 1.0)),
            (vec![r0, r1, c2, c3], slope_tone(base, -outward, 1.0)),
        ],
        gables: vec![((c1, c2), vec![c1, c2, r1]), ((c3, c0), vec![c3, c0, r0])],
        dormers: Vec::new(),
        ridge: Some((r0, r1)),
        ridge_offset: ridge,
    }
}

/// На какой доле высоты фронтона полувальма срезает его верх: ниже — стена,
/// выше — маленький скат.
const HALF_HIP_SPLIT: f32 = 0.55;

/// Полувальмовая: двускатная, у которой верх каждого фронтона срезан скатом
/// той же крутизны. `None` — дом слишком короткий, и срезы сошлись бы.
fn half_hip(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
) -> Option<GableRoof> {
    let [c0, c1, c2, c3] = frame.c;
    let t = HALF_HIP_SPLIT;
    // скат среза той же крутизны, что и боковой: по плану он уходит внутрь
    // на ту же долю полуширины, какую долю высоты он занимает
    let inset = (1.0 - t) * frame.width / 2.0;
    if 2.0 * inset >= frame.length - 1.0 {
        return None;
    }
    let ridge = ridge_lift(rise);
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let (s0, s1) = (r0 + frame.long * inset, r1 - frame.long * inset);
    // точки на скосах фронтона: линейная смесь в нарисованных координатах —
    // та же смесь плана и высоты, проекция аффинна
    let (p0, p1) = (c0.lerp(r0, t), c1.lerp(r1, t));
    let (p2, p3) = (c2.lerp(r1, t), c3.lerp(r0, t));
    let outward = frame.outward();
    Some(GableRoof {
        form: GableForm::HalfHip,
        slopes: vec![
            (vec![c0, c1, p1, s1, s0, p0], slope_tone(base, outward, 1.0)),
            (
                vec![c2, c3, p3, s0, s1, p2],
                slope_tone(base, -outward, 1.0),
            ),
            (vec![p1, p2, s1], slope_tone(base, frame.long, 1.2)),
            (vec![p3, p0, s0], slope_tone(base, -frame.long, 1.2)),
        ],
        gables: vec![
            ((c1, c2), vec![c1, c2, p2, p1]),
            ((c3, c0), vec![c3, c0, p0, p3]),
        ],
        dormers: Vec::new(),
        ridge: Some((s0, s1)),
        ridge_offset: ridge,
    })
}

/// Ломаная мансардная крыша: доля полуширины до излома, крутизна нижнего и
/// верхнего ската и потолок подъёма, настоящих метров. Нижний скат круче
/// жилой двускатной вдвое с лишним — это он делает под крышей комнату.
const GAMBREL_KNEE: f32 = 0.32;
const GAMBREL_LOWER_PITCH: f32 = 2.0;
const GAMBREL_UPPER_PITCH: f32 = 0.5;
const GAMBREL_RISE_MAX: f32 = 6.0;

/// Ломаная (мансардная) крыша: у каждого ската излом, фронтон пятиугольный.
fn gambrel(frame: &Frame, ridge_lift: impl Fn(f32) -> Vec2, base: Srgba) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let half = frame.width / 2.0;
    let knee = GAMBREL_KNEE * half * GAMBREL_LOWER_PITCH;
    let top = knee + (1.0 - GAMBREL_KNEE) * half * GAMBREL_UPPER_PITCH;
    let scale = (GAMBREL_RISE_MAX / top).min(1.0);
    let (knee_lift, ridge) = (ridge_lift(knee * scale), ridge_lift(top * scale));
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let k = GAMBREL_KNEE;
    let (k0, k1) = (c0.lerp(m0, k) + knee_lift, c1.lerp(m1, k) + knee_lift);
    let (k2, k3) = (c2.lerp(m1, k) + knee_lift, c3.lerp(m0, k) + knee_lift);
    let outward = frame.outward();
    GableRoof {
        form: GableForm::Gambrel,
        slopes: vec![
            (vec![c0, c1, k1, k0], slope_tone(base, outward, 1.5)),
            (vec![k0, k1, r1, r0], slope_tone(base, outward, 0.6)),
            (vec![r0, r1, k2, k3], slope_tone(base, -outward, 0.6)),
            (vec![k3, k2, c2, c3], slope_tone(base, -outward, 1.5)),
        ],
        gables: vec![
            ((c1, c2), vec![c1, c2, k2, r1, k1]),
            ((c3, c0), vec![c3, c0, k0, r0, k3]),
        ],
        dormers: Vec::new(),
        ridge: Some((r0, r1)),
        ridge_offset: ridge,
    }
}

/// Односкатная крыша сарая: подъём на метр ширины и его потолок, м.
const LEAN_TO_PITCH: f32 = 0.3;
const LEAN_TO_RISE_MAX: f32 = 2.0;

/// Односкатная: одна плоскость от низкой продольной стены к высокой; под
/// высоким краем встают треугольники торцов и полоса продольной стены.
/// `high_far` — поднят ли край `c2–c3` (иначе `c0–c1`).
fn lean_to(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    high_far: bool,
) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let up = ridge_lift((frame.width * LEAN_TO_PITCH).min(LEAN_TO_RISE_MAX));
    let outward = frame.outward();
    let (slope, tone, gables) = match high_far {
        true => (
            vec![c0, c1, c2 + up, c3 + up],
            slope_tone(base, outward, 1.0),
            vec![
                ((c1, c2), vec![c1, c2, c2 + up]),
                ((c3, c0), vec![c3, c0, c3 + up]),
                ((c2, c3), vec![c2, c3, c3 + up, c2 + up]),
            ],
        ),
        false => (
            vec![c0 + up, c1 + up, c2, c3],
            slope_tone(base, -outward, 1.0),
            vec![
                ((c1, c2), vec![c1, c2, c1 + up]),
                ((c3, c0), vec![c3, c0, c0 + up]),
                ((c0, c1), vec![c0, c1, c1 + up, c0 + up]),
            ],
        ),
    };
    GableRoof {
        form: GableForm::LeanTo,
        slopes: vec![(slope, tone)],
        gables,
        dormers: Vec::new(),
        ridge: None,
        ridge_offset: Vec2::ZERO,
    }
}

/// Выбор крыши для прямоугольного дома скатной когорты.
#[derive(Clone, Copy)]
pub(super) struct HouseRoof {
    pub(super) form: GableForm,
    /// Редкая вальма вместо всего остального.
    pub(super) hipped: bool,
    pub(super) dormers: bool,
    pub(super) seed: u32,
}

/// Формы крыши частного дома по десяткам: на снимке — сплошь двускатные,
/// изредка ломаная мансарда и полувальма.
const HOUSE_FORMS: [GableForm; 10] = [
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gambrel,
    GableForm::HalfHip,
];
/// Сараи: в основном одним скатом.
const SHED_FORMS: [GableForm; 10] = [
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
];
/// Вальма остаётся крупному дому и изредка: из ста больших домов — трём.
const HIP_HOUSE_AREA_MIN: f32 = 120.0;
const HIP_SHARE_OF_100: u32 = 3;
/// Сколько двускатных из десяти несут слуховые окна, и с какого размера дома.
const DORMER_SHARE: u32 = 3;
const DORMER_HOUSE_MIN_LENGTH: f32 = 8.0;
const DORMER_HOUSE_MIN_WIDTH: f32 = 7.0;
/// Ломаной и полувальме нужна ширина, иначе излом и срез не читаются, м.
const GAMBREL_MIN_WIDTH: f32 = 5.5;
const HALF_HIP_MIN_WIDTH: f32 = 4.5;

/// Какую крышу посев даёт прямоугольному дому.
pub(super) fn house_roof(building: &PolyArea, rect: &[Vec2; 4], seed: u32) -> HouseRoof {
    let seed = shape_seed(seed);
    let area = signed_ring_area(&building.outer).abs();
    let (length, width) = ((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length());
    let shed = building.building_use != BuildingUse::House && area <= SHED_FOOTPRINT_MAX;
    let table = match shed {
        true => &SHED_FORMS,
        false => &HOUSE_FORMS,
    };
    let form = match table[((seed >> 5) % 10) as usize] {
        GableForm::Gambrel if width < GAMBREL_MIN_WIDTH => GableForm::Gable,
        GableForm::HalfHip if width < HALF_HIP_MIN_WIDTH => GableForm::Gable,
        form => form,
    };
    HouseRoof {
        form,
        hipped: !shed && area >= HIP_HOUSE_AREA_MIN && (seed >> 9) % 100 < HIP_SHARE_OF_100,
        dormers: form == GableForm::Gable
            && length >= DORMER_HOUSE_MIN_LENGTH
            && width >= DORMER_HOUSE_MIN_WIDTH
            && (seed >> 16) % 10 < DORMER_SHARE,
        seed,
    }
}

/// Крыша прямоугольного дома выбранной формы. `None` — дом не из скатной
/// когорты или контур не держит прямоугольник.
pub(super) fn house_gable(
    building: &PolyArea,
    class: &BuildingClass,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    choice: HouseRoof,
    fp: Option<HouseFootprint>,
) -> Option<GableRoof> {
    if !class.is_pitched() {
        return None;
    }
    let rect = gable_rect(building, fp)?;
    let frame = Frame::of(rect, lift);
    let rise = ridge_rise(frame.width);
    let mut roof = match choice.form {
        GableForm::HalfHip => half_hip(&frame, &ridge_lift, base, rise)
            .unwrap_or_else(|| plain_gable(&frame, &ridge_lift, base, rise)),
        GableForm::Gambrel => gambrel(&frame, &ridge_lift, base),
        GableForm::LeanTo => lean_to(&frame, &ridge_lift, base, (choice.seed >> 13) & 1 == 1),
        GableForm::Gable | GableForm::Cross => plain_gable(&frame, &ridge_lift, base, rise),
    };
    if choice.dormers && roof.form == GableForm::Gable {
        roof.dormers = dormers(&frame, &ridge_lift, base, rise, choice.seed);
    }
    Some(roof)
}

/// Слуховое окно: отступ передней стенки от карниза в долях полуширины,
/// высота стенки над скатом, крутизна его крыши, м.
const DORMER_SETBACK: f32 = 0.3;
const DORMER_FRONT: f32 = 0.9;
const DORMER_PITCH: f32 = 0.8;
/// Ширина окна — доля длины дома, зажатая в метрах.
const DORMER_WIDTH_SHARE: f32 = 0.22;
const DORMER_WIDTH: std::ops::RangeInclusive<f32> = 1.6..=2.6;

/// Слуховые окна на одном скате двускатной крыши: передняя стенка с окном,
/// щёчки и своя маленькая двускатная, упёртая коньком в скат. В плоских
/// режимах их нет — без подъёма это была бы заплата на скате.
fn dormers(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
    seed: u32,
) -> Vec<DormerFace> {
    let [c0, _, _, c3] = frame.c;
    let half = frame.width / 2.0;
    if ridge_lift(1.0) == Vec2::ZERO || half <= 0.0 {
        return Vec::new();
    }
    let pitch = rise / half;
    let setback = DORMER_SETBACK * half;
    let (z0, z1) = (setback * pitch, setback * pitch + DORMER_FRONT);
    let wide =
        (frame.length * DORMER_WIDTH_SHARE).clamp(*DORMER_WIDTH.start(), *DORMER_WIDTH.end());
    let z_ridge = z1 + wide / 2.0 * DORMER_PITCH;
    if z_ridge > rise - 0.3 {
        return Vec::new();
    }
    // скат выбирает посев: вход в дом бывает с любой стороны
    let across = (c3 - c0).normalize_or_zero();
    let (origin, inward) = match (seed >> 14) & 1 {
        0 => (c0, across),
        _ => (c3, -across),
    };
    let at = |u: f32, v: f32, z: f32| origin + frame.long * u + inward * v + ridge_lift(z);
    let (eave, ridge) = (z1 / pitch, z_ridge / pitch);
    // окно одно: у одноэтажного дома мансарда одна, а ряд слуховых окон —
    // примета многоэтажного, по окну на подъезд. Не посередине, а где выпало
    let shift = ((seed >> 20) & 0xff) as f32 / 255.0;
    let centre = frame.length * (0.35 + 0.3 * shift);
    let front = -inward;
    let (u0, u1) = (centre - wide / 2.0, centre + wide / 2.0);
    let (f0, f1) = (at(u0, setback, z0), at(u1, setback, z0));
    let (t0, t1) = (at(u0, setback, z1), at(u1, setback, z1));
    let apex = at(centre, setback, z_ridge);
    let (e0, e1) = (at(u0, eave, z1), at(u1, eave, z1));
    let back = at(centre, ridge, z_ridge);
    let pane = wide * 0.3;
    vec![
        DormerFace::Wall(vec![f0, e0, t0], -frame.long),
        DormerFace::Wall(vec![f1, t1, e1], frame.long),
        DormerFace::Wall(vec![f0, f1, t1, apex, t0], front),
        DormerFace::Glass(
            vec![
                at(centre - pane, setback, z0 + 0.2),
                at(centre + pane, setback, z0 + 0.2),
                at(centre + pane, setback, z1 - 0.15),
                at(centre - pane, setback, z1 - 0.15),
            ],
            front,
        ),
        DormerFace::Roof(vec![t0, apex, back, e0], slope_tone(base, -frame.long, 1.0)),
        DormerFace::Roof(vec![apex, t1, e1, back], slope_tone(base, frame.long, 1.0)),
    ]
}

/// Крестовая двускатная над домом из нескольких прямоугольников — буквой Г,
/// Т, П. Контур режется на прямоугольники тем же разрезом, что и гаражная
/// кровля ([`crate::map::buildings::garages::split_rings`]); самый крупный
/// кусок — корпус под обычной двускатной вдоль своей длинной стороны,
/// остальные — крылья, каждое под своей двускатной, пристроенной к уже
/// накрытому соседу:
///
/// * крыло у **длинной** стороны соседа идёт коньком поперёк неё, и конёк
///   уходит в скат соседа до ендовы;
/// * крыло у **торца** соседа продолжает его конёк, своим торцом упираясь в
///   его фронтон.
///
/// `None` — контур на прямоугольники не режется, кусок не пристроить ни к
/// кому или крыло шире соседа (его конёк встал бы выше соседского).
pub(super) fn cross_gable(
    building: &PolyArea,
    class: &BuildingClass,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    if !class.is_pitched() {
        return None;
    }
    let ring = merge_close_points(&building.outer, true, 0.2);
    // разрезов у Г два, у Т и П больше, и крылья пристраиваются не при
    // всяком: у Г, разрезанной «не той» хордой, крыло встаёт к торцу корпуса и
    // свешивается с него. Перебираются все разбиения, начиная с крупного корпуса
    let mut candidates: Vec<(f32, Vec<[Vec2; 4]>)> = rect_splits(ring, CROSS_PIECES_MAX)
        .into_iter()
        .filter(|pieces| pieces.len() >= 2)
        .filter_map(|pieces| {
            let rects: Vec<[Vec2; 4]> = pieces
                .iter()
                .map(|piece| min_area_rect(piece))
                .collect::<Option<_>>()?;
            // кусок, недотягивающий до прямоугольника, накрылся бы крышей с
            // торчащим углом — ровно то, от чего двускатная отказывает контуру
            let rectangular = pieces.iter().zip(&rects).all(|(piece, rect)| {
                signed_ring_area(piece).abs() >= RECT_FILL_MIN * rect_area(rect)
            });
            let largest = rects.iter().map(rect_area).fold(0.0, f32::max);
            rectangular.then_some((largest, rects))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    candidates
        .into_iter()
        .find_map(|(_, rects)| cross_over(rects, lift, &ridge_lift, base))
}

/// Все разбиения кольца на куски не больше `budget` штук: из каждой вогнутой
/// вершины — оба разреза по её стенам
/// ([`crate::map::buildings::garages::reflex_cuts`]), и дальше рекурсивно.
/// Почти прямоугольник не режется.
///
/// `SPLITS_MAX` — настоящий предохранитель, а не фильтр вывода: набрав потолок,
/// перебор **бросается целиком** (`break 'cuts`), и оставшиеся вершины даже не
/// режутся. Вывод от этого не меняется — всё, что шло после потолка, и так
/// отбрасывалось.
fn rect_splits(ring: Vec<Vec2>, budget: usize) -> Vec<Vec<Vec<Vec2>>> {
    // кольцо здесь слито из близких вершин и нарезано кусками — не контур
    // дома, и пятна у него нет: заполнение считается по нему самому
    let fill = min_area_rect(&ring)
        .and_then(|rect| {
            let area = rect_area(&rect);
            (area > 0.0).then(|| signed_ring_area(&ring).abs() / area)
        })
        .unwrap_or(0.0);
    if budget <= 1 || ring.len() < 5 || fill >= SPLIT_FILL_DONE {
        return vec![vec![ring]];
    }
    let mut result = Vec::new();
    'cuts: for (near, far) in crate::map::buildings::garages::reflex_cuts(&ring) {
        // разбиения дальнего куска считаются один раз на каждый остаток
        // бюджета: `budget - left.len()` пробегает не больше `budget - 1`
        // разных значений, а на типичном контуре — одно, и без этого весь
        // перебор дальнего куска повторялся бы на каждый `left`
        let mut far_splits: Vec<Option<Vec<Vec<Vec<Vec2>>>>> = vec![None; budget];
        for left in rect_splits(near, budget - 1) {
            let rest = budget - left.len();
            let rights = far_splits[rest].get_or_insert_with(|| rect_splits(far.clone(), rest));
            for right in rights.iter() {
                if result.len() >= SPLITS_MAX {
                    break 'cuts;
                }
                result.push(left.iter().cloned().chain(right.iter().cloned()).collect());
            }
        }
    }
    match result.is_empty() {
        true => vec![vec![ring]],
        false => result,
    }
}

/// Заполнение, с которого кусок уже не режется, и потолок числа разбиений —
/// у Т и П они повторяются, а считать одно и то же дважды незачем.
const SPLIT_FILL_DONE: f32 = 0.97;
const SPLITS_MAX: usize = 24;

/// Крестовая двускатная над уже разрезанным контуром; `None` — какой-то
/// кусок не пристроить.
fn cross_over(
    mut rects: Vec<[Vec2; 4]>,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    let largest =
        (0..rects.len()).max_by(|a, b| rect_area(&rects[*a]).total_cmp(&rect_area(&rects[*b])))?;
    let main = Piece::of_rect(rects.swap_remove(largest));
    let main_rise = ridge_rise(main.width());
    let mut roof = plain_gable(&Frame::of(main.rect(), lift), &ridge_lift, base, main_rise);
    roof.form = GableForm::Cross;

    // крылья пристраиваются к уже накрытым кускам, пока пристраивается хоть одно
    let mut placed = vec![(main, main_rise)];
    while !rects.is_empty() {
        let mut attached = None;
        'search: for (index, rect) in rects.iter().enumerate() {
            for (parent, parent_rise) in &placed {
                if let Some(wing) = Wing::attach(parent, *parent_rise, rect) {
                    attached = Some((index, wing));
                    break 'search;
                }
            }
        }
        let (index, wing) = attached?;
        rects.swap_remove(index);
        placed.push(wing.push(&mut roof, lift, &ridge_lift, base));
    }
    Some(roof)
}

/// Больше кусков — это уже не дом с крыльями, а гребёнка, и вальма над ней
/// честнее пучка коньков.
const CROSS_PIECES_MAX: usize = 4;
/// Насколько стороны куска могут разойтись со сторонами соседа, м: обводка
/// частного дома неровная, и прямоугольники кусков стоят не идеально.
const CROSS_TOLERANCE: f32 = 0.6;

/// Прямоугольный кусок крыши в собственной системе: `d` — вдоль конька, `n` —
/// поперёк (CCW), пролёты `u` вдоль и `v` поперёк от `origin`.
#[derive(Clone, Copy)]
struct Piece {
    origin: Vec2,
    d: Vec2,
    n: Vec2,
    u: (f32, f32),
    v: (f32, f32),
}

impl Piece {
    /// Кусок по `min_area_rect`: конёк вдоль длинной стороны.
    fn of_rect(rect: [Vec2; 4]) -> Self {
        let d = (rect[1] - rect[0]).normalize_or_zero();
        Self {
            origin: rect[0],
            d,
            n: d.perp(),
            u: (0.0, (rect[1] - rect[0]).length()),
            v: (0.0, (rect[2] - rect[1]).length()),
        }
    }

    fn width(&self) -> f32 {
        self.v.1 - self.v.0
    }

    fn at(&self, u: f32, v: f32) -> Vec2 {
        self.origin + self.d * u + self.n * v
    }

    /// Прямоугольник CCW, `c0→c1` вдоль конька.
    fn rect(&self) -> [Vec2; 4] {
        [
            self.at(self.u.0, self.v.0),
            self.at(self.u.1, self.v.0),
            self.at(self.u.1, self.v.1),
            self.at(self.u.0, self.v.1),
        ]
    }

    /// Пролёты прямоугольника в системе этого куска — `None`, если его стороны
    /// не параллельны сторонам куска.
    fn project(&self, rect: &[Vec2; 4]) -> Option<((f32, f32), (f32, f32))> {
        let points: Vec<(f32, f32)> = rect
            .iter()
            .map(|p| {
                (
                    (*p - self.origin).dot(self.d),
                    (*p - self.origin).dot(self.n),
                )
            })
            .collect();
        let fold = |pick: fn(&(f32, f32)) -> f32| {
            points
                .iter()
                .map(pick)
                .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
        };
        let (u, v) = (fold(|p| p.0), fold(|p| p.1));
        // у повёрнутого прямоугольника углы не лягут на углы своей рамки
        let aligned = points.iter().all(|(pu, pv)| {
            ((pu - u.0).abs() < CROSS_TOLERANCE || (pu - u.1).abs() < CROSS_TOLERANCE)
                && ((pv - v.0).abs() < CROSS_TOLERANCE || (pv - v.1).abs() < CROSS_TOLERANCE)
        });
        aligned.then_some((u, v))
    }
}

/// Как крыло стоит к соседу.
enum Wing {
    /// У длинной стороны: `side` — знак стороны по `n` соседа.
    Side {
        parent: Piece,
        parent_rise: f32,
        side: f32,
        u: (f32, f32),
        far: f32,
    },
    /// У торца: крыло продолжает конёк соседа; `side` — знак торца по `d`.
    End {
        parent: Piece,
        side: f32,
        u: (f32, f32),
        v: (f32, f32),
    },
}

impl Wing {
    /// Пристраивается ли прямоугольник к соседу, и как.
    fn attach(parent: &Piece, parent_rise: f32, rect: &[Vec2; 4]) -> Option<Self> {
        let (u, v) = parent.project(rect)?;
        let t = CROSS_TOLERANCE;
        let width = parent.width();
        let within = |range: (f32, f32), of: (f32, f32)| range.0 >= of.0 - t && range.1 <= of.1 + t;
        if within(u, parent.u) && (u.1 - u.0) <= width * 1.05 {
            if (v.0 - parent.v.1).abs() < t {
                return Some(Self::Side {
                    parent: *parent,
                    parent_rise,
                    side: 1.0,
                    u,
                    far: v.1,
                });
            }
            if (v.1 - parent.v.0).abs() < t {
                return Some(Self::Side {
                    parent: *parent,
                    parent_rise,
                    side: -1.0,
                    u,
                    far: v.0,
                });
            }
        }
        if within(v, parent.v) {
            if (u.0 - parent.u.1).abs() < t {
                return Some(Self::End {
                    parent: *parent,
                    side: 1.0,
                    u: (parent.u.1, u.1),
                    v,
                });
            }
            if (u.1 - parent.u.0).abs() < t {
                return Some(Self::End {
                    parent: *parent,
                    side: -1.0,
                    u: (u.0, parent.u.0),
                    v,
                });
            }
        }
        None
    }

    /// Скаты и торец крыла — в крышу; наружу — кусок крыла и его подъём, к
    /// которым пристраиваются следующие.
    fn push(
        &self,
        roof: &mut GableRoof,
        lift: Vec2,
        ridge_lift: impl Fn(f32) -> Vec2,
        base: Srgba,
    ) -> (Piece, f32) {
        match *self {
            Self::Side {
                parent,
                parent_rise,
                side,
                u,
                far,
            } => {
                let line = match side > 0.0 {
                    true => parent.v.1,
                    false => parent.v.0,
                };
                let rise = ridge_rise(u.1 - u.0).min(parent_rise);
                // конёк крыла уходит в скат соседа, пока не упрётся в него: на
                // глубину своего подъёма, делённого на крутизну соседа
                let half = parent.width() / 2.0;
                let reach = (rise / (parent_rise / half).max(1e-3)).min(half);
                let at = |u: f32, v: f32| parent.at(u, v) + lift;
                let mid = (u.0 + u.1) / 2.0;
                let up = ridge_lift(rise);
                let (j0, j1) = (at(u.0, line), at(u.1, line));
                let (f0, f1) = (at(u.0, far), at(u.1, far));
                let far_ridge = at(mid, far) + up;
                let valley = at(mid, line - side * reach) + up;
                roof.slopes.push((
                    vec![j0, f0, far_ridge, valley],
                    slope_tone(base, -parent.d, 1.0),
                ));
                roof.slopes.push((
                    vec![f1, j1, valley, far_ridge],
                    slope_tone(base, parent.d, 1.0),
                ));
                // правый перпендикуляр ребра торца смотрит наружу, от соседа
                let (a, b) = match side > 0.0 {
                    true => (f1, f0),
                    false => (f0, f1),
                };
                roof.gables.push(((a, b), vec![a, b, far_ridge]));
                // сам кусок крыла: конёк от соседа наружу
                let d = parent.n * side;
                let origin = parent.at(u.0, line);
                let spread = (u.1 - u.0) * side;
                let piece = Piece {
                    origin,
                    d,
                    n: d.perp(),
                    u: (0.0, (far - line).abs()),
                    v: match spread > 0.0 {
                        true => (-spread, 0.0),
                        false => (0.0, -spread),
                    },
                };
                (piece, rise)
            }
            Self::End { parent, side, u, v } => {
                let piece = Piece { u, v, ..parent };
                let rise = ridge_rise(piece.width());
                let mut wing = plain_gable(&Frame::of(piece.rect(), lift), &ridge_lift, base, rise);
                // торец у соседа упирается в его стену: фронтона на нём нет
                let inner = match side > 0.0 {
                    true => 1,
                    false => 0,
                };
                wing.gables.remove(inner);
                roof.slopes.append(&mut wing.slopes);
                roof.gables.append(&mut wing.gables);
                (piece, rise)
            }
        }
    }
}
