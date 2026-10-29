use bevy::prelude::*;

use crate::map::osm::{Highway, RoadClass, RoadLine};

use super::{
    DIRT_RADIUS, DRIVE_RADIUS, MAJOR_RADIUS, NOSE_MAX_ANGLE, NOSE_MIN_ANGLE, NOSE_RADIUS,
    NOSE_REACH, NOSE_SHARE, PATH_RADIUS, STREET_RADIUS, ccw_angle,
};

/// Одна дорога, выходящая из узла.
pub(super) struct Arm<'d> {
    pub(super) class: RoadClass,
    pub(super) highway: Highway,
    /// Дуга кольца (`Drawn::on_ring`): у колец клинья штрихуют свои островки
    /// (`roads/gores.rs`), и клина развилки ([`fork_gore`]) им не нужно.
    pub(super) ring: bool,
    /// Мощёная дорожка ([`RoadLine::is_paved_path`]): её скругления ложатся
    /// в слой тротуаров, а не тропинок.
    ///
    /// [`RoadLine::is_paved_path`]: crate::map::osm::RoadLine::is_paved_path
    pub(super) paved: bool,
    /// Грунтовая улица ([`RoadLine::is_unpaved_street`]): угол двух таких
    /// ложится в слой грунтовок, а не асфальта.
    ///
    /// [`RoadLine::is_unpaved_street`]: crate::map::osm::RoadLine::is_unpaved_street
    pub(super) unpaved: bool,
    /// Полуширина слева и справа по ходу луча. Они разные под клином на
    /// одну сторону (`roads/tapers.rs`): сужаемая кромка в узле стоит между
    /// полуширинами узкого соседа и своей — у шва на соседской, — а
    /// сохранённая на своей.
    pub(super) half: [f32; 2],
    /// Тротуар слева и справа по ходу луча: у половины разделённой улицы со
    /// стороны пары его нет.
    pub(super) sidewalk: [Option<f32>; 2],
    /// Обочина до отдельного тротуара слева и справа по ходу луча
    /// (`RoadLine::verges`, `Drawn::verges_drawn`).
    pub(super) verge: [Option<f32>; 2],
    pub(super) direction: Vec2,
    /// Сколько метров край ленты идёт прямо — до следующей вершины.
    pub(super) run: f32,
    /// Где луч пересекает другая дорога — ближайший общий узел, м по оси от
    /// узла; нет такого в досягаемости хвоста площадки — бесконечность.
    /// Площадка плитки у бордюра ([`kerb_pad`]) за него не тянется.
    pub(super) crossed: f32,
    /// Дорога и её торец (`0` — начало, `1` — конец), если луч — торец пути.
    pub(super) end: Option<(usize, usize)>,
    /// Направление исходного крайнего звена оси у торца, от узла в луч
    /// ([`meshing::butt_direction`]): прямой торец ленты идёт поперёк него,
    /// и по нему строится сторона наружного угла ([`outer_corner`]) — по
    /// направлению луча (`direction`, на вершину ленты после слияния) она
    /// отходила от торцевой кромки, и между ними оставалась щель. Только у
    /// торцевого луча; у сквозного торца нет.
    pub(super) butt: Option<Vec2>,
    /// Клин дальше по лучу ([`FarTaper`]), если он есть.
    pub(super) far: Option<FarTaper>,
    /// Своя полуширина дороги.
    pub(super) full: f32,
    /// Полуширина каждой стороны, к которой кромка под клином приходит за
    /// `widening` метров по лучу: своя — от шва, соседская — к шву; дальше
    /// она держится ([`Arm::half_at`]).
    pub(super) reach: [f32; 2],
    pub(super) widening: f32,
    /// Наклон кромки каждой стороны в клине: на сколько она отходит от оси
    /// на метр луча. Вне клина — ноль; скругление ставит дугу на эту,
    /// наклонную, кромку ([`fillet_arc`]).
    pub(super) slope: [f32; 2],
    /// Узловая осевая дороги, вершина узла на ней и куда по ней идёт луч —
    /// из них [`Arm::trail`].
    pub(super) path: &'d [Vec2],
    pub(super) vertex: usize,
    pub(super) forward: bool,
}

impl Arm<'_> {
    /// Осевая луча от узла на [`NOSE_REACH`] вперёд — по ней ищется нос: у
    /// острой развилки кромки сходятся за десятки метров от узла, где ленты
    /// давно уже не прямые (съезд с кольца, ветка по дуге). Собирается только
    /// для носа: лучей в городе десятки тысяч, носов — сотни. У замкнутого
    /// кольца — через шов.
    pub(super) fn trail(&self) -> Vec<Vec2> {
        let path = self.path;
        let last = path.len() - 1;
        let closed = path[0] == path[last];
        let step = |from: usize| {
            if closed {
                Some(if self.forward {
                    (from + 1) % last
                } else {
                    (from + last - 1) % last
                })
            } else if self.forward {
                (from < last).then_some(from + 1)
            } else {
                from.checked_sub(1)
            }
        };
        let mut trail = vec![path[self.vertex]];
        let (mut reach, mut from) = (0.0, self.vertex);
        while reach < NOSE_REACH {
            let Some(index) = step(from).filter(|&index| index != self.vertex) else {
                break;
            };
            reach += path[index].distance(path[from]);
            trail.push(path[index]);
            from = index;
        }
        trail
    }

    /// Полуширина стороны `side` в `along` метрах от узла по лучу: в клине
    /// кромка расходится линейно, как её кладет лента
    /// (`MeshBuilder::push_taper_sided`); за узлом — сужается дальним клином
    /// ([`FarTaper`], [`Arm::far`]).
    pub(super) fn half_at(&self, side: usize, along: f32) -> f32 {
        let mut half = if self.widening <= 0.0 {
            self.half[side]
        } else {
            let share = (along / self.widening).min(1.0);
            self.half[side] + (self.reach[side] - self.half[side]) * share
        };
        if let Some(far) = self.far
            && far.sides[side]
            && along > far.start
        {
            let share = ((along - far.start) / far.length).min(1.0);
            half += (far.half - half) * share;
        }
        half
    }
}

/// Клин дальше по лучу: узел вне его, но кромка за узлом сужается к дальнему
/// шву way. Нос ищет кромки по [`Arm::trail`] за десятки метров от узла, и по
/// номинальной полуширине луча вставал мимо настоящего расхождения кромок —
/// штрихуемый клин ложился на плитку обочины (Тула, устье съезда с Фрунзе,
/// S7).
#[derive(Clone, Copy)]
pub(super) struct FarTaper {
    /// Сколько метров от узла до начала клина.
    pub(super) start: f32,
    /// Длина клина — обрезанная, как её положила лента (`tapers::fit`).
    pub(super) length: f32,
    /// Полуширина узкого соседа.
    pub(super) half: f32,
    /// Какие стороны луча ([`Arm::half`] — слева, справа) клин сужает.
    pub(super) sides: [bool; 2],
}

/// Радиус бордюра между двумя лучами узла — по младшему классу пары (см.
/// [`MAJOR_RADIUS`]); между двумя грунтовками — не больше [`DIRT_RADIUS`].
pub(super) fn kerb_radius(first: &Arm, second: &Arm) -> f32 {
    let radius = class_radius(first).min(class_radius(second));
    if first.unpaved && second.unpaved {
        radius.min(DIRT_RADIUS)
    } else {
        radius
    }
}

/// Острый ли угол от `first` до `second` для носа ([`NOSE_SHARE`]) — если
/// скругление там не легло.
pub(super) fn is_nose(first: &Arm, second: &Arm) -> bool {
    (NOSE_MIN_ANGLE..NOSE_MAX_ANGLE).contains(&ccw_angle(first, second))
}

/// Радиус носа при радиусе пары `radius` (уже умноженном на `scale`).
pub(super) fn nose_radius(radius: f32, scale: f32) -> f32 {
    (radius * NOSE_SHARE).min(NOSE_RADIUS * scale)
}

fn class_radius(arm: &Arm) -> f32 {
    radius_of(arm.class, arm.highway)
}

/// Радиус бордюра дороги по её классу — без ручки `Corner radius`. Им клин
/// у шва решает, прикрывает ли примыкание уступ (`roads/tapers.rs`).
pub(in crate::map::roads) fn road_radius(road: &RoadLine) -> f32 {
    radius_of(road.class, road.highway)
}

fn radius_of(class: RoadClass, highway: Highway) -> f32 {
    if class == RoadClass::Alley {
        return PATH_RADIUS;
    }
    match highway {
        Highway::Motorway
        | Highway::Trunk
        | Highway::Primary
        | Highway::Secondary
        | Highway::MotorwayLink
        | Highway::TrunkLink
        | Highway::PrimaryLink
        | Highway::SecondaryLink => MAJOR_RADIUS,
        Highway::Tertiary
        | Highway::TertiaryLink
        | Highway::Residential
        | Highway::Unclassified => STREET_RADIUS,
        // жилая зона, проезд и переезд через тротуар (дорожка, нарисованная
        // асфальтом, — `network::driveway_crossings`)
        Highway::LivingStreet | Highway::Service | Highway::Path => DRIVE_RADIUS,
    }
}
