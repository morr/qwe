//! Как нарисована одна машина.
//!
//! Сверху легковой автомобиль — это не прямоугольник, а **силуэт со
//! скруглёнными углами и тёмной кабиной посередине**: лобовое стекло,
//! крыша, заднее стекло. Именно эти три поперечные полосы и делают
//! пятно на асфальте машиной; цвет кузова — уже второе. Прямоугольник
//! читался как коробка ровно потому, что у него нет ни одной из них.
//!
//! Все размеры кузова заданы **долями его габарита**, а не метрами: тип
//! кузова меняет длину и ширину ([`CarShape`]), а разметка капота, крыши и
//! багажника при этом остаётся своей у каждого типа — у седана длинный
//! багажник, у хэтчбека от него остаётся свес за задним стеклом, у фургона
//! крыша начинается сразу за лобовым стеклом.
//!
//! В метрах багажник (`(backlight + 0.5) * length`) выходит такой: седан
//! 0.66, хэтчбек 0.43, кроссовер 0.41, универсал 0.23, фургон 0.16. То есть
//! «у хэтчбека багажника нет вовсе» — преувеличение: у него две трети
//! седанского, и нулевого багажника таблица не допускает вовсе
//! (`the_cabin_runs_from_nose_to_tail_in_order` требует `backlight > -0.5`).
//! И различие это **ближнего зума**: ступень `Full` тянется от `MIN_ZOOM`
//! 0.05 м/px до `CAR_DETAIL_MAX_ZOOM` 0.18, и седан против хэтчбека — это
//! 13 пикселей багажника против 9 у ближнего края и 3.7 против 2.4 у
//! дальнего, где кабиной различимы уже только фургон и, слабее, универсал.
//!
//! Подробность — [`CarDetail`], её выбирает ступень зума слоя
//! ([`super::CarLods`]). Дальше всех стоит `Block`, тот самый прямоугольник:
//! на пяти пикселях длины ни скругление, ни стёкла не читаются, а вершины
//! стоят столько же, сколько вблизи.

use bevy::prelude::*;

use crate::map::SHADOW_COLOR;
use crate::map::meshing::MeshBuilder;

/// Тип кузова. Доли в списке — то, как часто он встречается во дворе
/// русского города: половина — седаны и хэтчбеки, кроссоверов заметно
/// меньше, фургон один на десяток.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CarShape {
    Sedan,
    Hatch,
    Wagon,
    Crossover,
    Van,
}

/// Десять слотов — чтобы таблица читалась процентами, как палитра кузовов и
/// таблицы кровель.
const SHAPES: [CarShape; 10] = [
    CarShape::Sedan,
    CarShape::Sedan,
    CarShape::Sedan,
    CarShape::Hatch,
    CarShape::Hatch,
    CarShape::Hatch,
    CarShape::Wagon,
    CarShape::Crossover,
    CarShape::Crossover,
    CarShape::Van,
];

/// Палитра кузовов, по долям близкая к тому, что видно на снимке русского
/// города. Слот выбирается равномерно, поэтому доля цвета — это счёт слотов:
/// светлого ахроматического (белый, серебро, серый, тёмно-серый) — две пятых,
/// чёрного — пятая часть, остальное цветное.
const COLORS: [Color; 10] = [
    Color::srgb(0.78, 0.78, 0.77),
    Color::srgb(0.72, 0.73, 0.74),
    Color::srgb(0.60, 0.61, 0.62),
    Color::srgb(0.46, 0.47, 0.48),
    Color::srgb(0.16, 0.16, 0.17),
    Color::srgb(0.20, 0.20, 0.21),
    Color::srgb(0.22, 0.27, 0.38),
    Color::srgb(0.45, 0.14, 0.13),
    Color::srgb(0.29, 0.33, 0.28),
    Color::srgb(0.55, 0.50, 0.42),
];

/// Цвет кузова по случайной доле `[0, 1)`.
pub fn color_from_share(share: f32) -> Color {
    let index = (share * COLORS.len() as f32) as usize;
    COLORS[index.min(COLORS.len() - 1)]
}

impl CarShape {
    /// Слот по случайной доле `[0, 1)`.
    pub fn from_share(share: f32) -> Self {
        let index = (share * SHAPES.len() as f32) as usize;
        SHAPES[index.min(SHAPES.len() - 1)]
    }

    /// Габариты и разметка кузова этого типа.
    fn profile(self) -> Profile {
        match self {
            // Логан: длинный багажник, лобовое за серединой
            CarShape::Sedan => Profile {
                length: 4.4,
                width: 1.8,
                nose: 0.58,
                tail: 0.64,
                windshield: 0.15,
                roof_front: -0.04,
                roof_back: -0.24,
                backlight: -0.35,
            },
            // Калина: багажник — свес за задним стеклом, само стекло почти на корме
            CarShape::Hatch => Profile {
                length: 3.9,
                width: 1.72,
                nose: 0.58,
                tail: 0.66,
                windshield: 0.17,
                roof_front: -0.02,
                roof_back: -0.26,
                backlight: -0.39,
            },
            // Универсал: крыша тянется до самой кормы
            CarShape::Wagon => Profile {
                length: 4.6,
                width: 1.8,
                nose: 0.58,
                tail: 0.70,
                windshield: 0.16,
                roof_front: -0.03,
                roof_back: -0.36,
                backlight: -0.45,
            },
            // Кроссовер: шире и «квадратнее» легковой
            CarShape::Crossover => Profile {
                length: 4.5,
                width: 1.86,
                nose: 0.62,
                tail: 0.70,
                windshield: 0.17,
                roof_front: -0.02,
                roof_back: -0.29,
                backlight: -0.41,
            },
            // Газель: кабина у самого носа, дальше — глухой фургон
            CarShape::Van => Profile {
                length: 5.3,
                width: 1.95,
                nose: 0.74,
                tail: 0.84,
                windshield: 0.34,
                roof_front: 0.18,
                roof_back: -0.42,
                backlight: -0.47,
            },
        }
    }

    /// Длина кузова, м — по ней слой считает, не наехала ли машина на
    /// соседнюю на изломе улицы.
    pub fn length(self) -> f32 {
        self.profile().length
    }

    /// Ширина кузова, м — по ней ряд отступает от кромки проезжей части, так
    /// что фургон стоит к бордюру так же вплотную, как седан.
    pub fn width(self) -> f32 {
        self.profile().width
    }

    /// Высота, м: длина тени машины считается тем же котангенсом высоты
    /// солнца, что у домов, и фургон отбрасывает её заметно длиннее.
    pub fn height(self) -> f32 {
        match self {
            CarShape::Sedan | CarShape::Hatch | CarShape::Wagon => 1.5,
            CarShape::Crossover => 1.7,
            CarShape::Van => 2.3,
        }
    }
}

/// Разметка кузова в долях: `length`/`width` в метрах, всё остальное — доля
/// длины (x, нос `+0.5`) или доля полуширины (y).
struct Profile {
    length: f32,
    width: f32,
    /// Полуширина торцов: у легковой машины нос и корма заметно у́же миделя,
    /// у фургона — почти нет.
    nose: f32,
    tail: f32,
    /// Передняя кромка лобового стекла (там, где кончается капот).
    windshield: f32,
    /// Стык лобового с крышей и крыши с задним стеклом.
    roof_front: f32,
    roof_back: f32,
    /// Задняя кромка заднего стекла (там, где начинается багажник).
    backlight: f32,
}

/// Полуширина кабины у переднего края лобового стекла и у крыши: сверху
/// остекление сужается к крыше, а по бокам от него остаются стойки и двери
/// цвета кузова.
const CABIN_WIDE: f32 = 0.78;
const CABIN_NARROW: f32 = 0.66;
/// Насколько зеркало торчит за габарит и какой оно длины (доли полуширины и
/// длины). Пара зеркал — самый дешёвый признак того, что у пятна есть перёд.
const MIRROR_REACH: f32 = 1.22;
const MIRROR_LONG: f32 = 0.045;

/// Лобовое стекло сверху светлее заднего: в него смотрит небо. Обе краски
/// холодные и почти без цвета — стекло не красят.
const GLASS_FRONT: Color = Color::srgb(0.34, 0.37, 0.43);
const GLASS_BACK: Color = Color::srgb(0.19, 0.20, 0.23);
/// Крыша смотрит прямо в небо, поэтому она — самое светлое место кузова;
/// зеркало, наоборот, всегда в собственной тени.
const ROOF_LIGHTEN: f32 = 0.12;
const MIRROR_DARKEN: f32 = 0.32;

/// Насколько подробно рисуется машина. Одна ступень — одна ступень зума
/// слоя, и они идут только на убывание: ближе `Full`, дальше `Silhouette`,
/// у горизонта `Block`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CarDetail {
    /// Кузов, стёкла, крыша, зеркала.
    Full,
    /// Один силуэт кузова: скругления ещё видны, стёкла уже нет.
    Silhouette,
    /// Прямоугольник габарита — восемь вершин на машину вместе с тенью.
    Block,
}

/// Машина на своём месте: центр, направление вдоль кузова, цвет и тип.
pub struct Car {
    pub at: Vec2,
    pub along: Vec2,
    pub color: Color,
    pub shape: CarShape,
}

impl Car {
    /// Точка кузова по долям: `x` вдоль (нос `+0.5`), `y` поперёк (доля
    /// полуширины, влево положительна). `offset` сдвигает всю машину — так
    /// рисуется её тень.
    fn point(&self, profile: &Profile, offset: Vec2, x: f32, y: f32) -> Vec2 {
        self.at
            + offset
            + self.along * (x * profile.length)
            + self.along.perp() * (y * profile.width / 2.0)
    }

    /// Контур кузова: правый борт от носа к корме, потом левый обратно.
    /// Шесть точек на борт — скруглений ровно столько, сколько видно на
    /// экране в самом ближнем бакете, где машина длиной под сотню пикселей.
    fn outline(&self, profile: &Profile, offset: Vec2) -> Vec<Vec2> {
        let side = [
            (0.500, profile.nose),
            (0.455, 0.93),
            (0.330, 1.00),
            (-0.330, 1.00),
            (-0.455, 0.94),
            (-0.500, profile.tail),
        ];
        let mut points = Vec::with_capacity(side.len() * 2);
        for &(x, y) in &side {
            points.push(self.point(profile, offset, x, y));
        }
        for &(x, y) in side.iter().rev() {
            points.push(self.point(profile, offset, x, -y));
        }
        points
    }

    /// Прямоугольник габарита — им рисуется и дальний бакет, и тень под ним.
    fn block(&self, profile: &Profile, offset: Vec2) -> [Vec2; 4] {
        [
            self.point(profile, offset, -0.5, -1.0),
            self.point(profile, offset, 0.5, -1.0),
            self.point(profile, offset, 0.5, 1.0),
            self.point(profile, offset, -0.5, 1.0),
        ]
    }
}

/// Тень машины: тот же силуэт, сдвинутый по свету. На `Block` — габаритный
/// прямоугольник: тень размыта сама по себе, и скругление в ней не читается
/// уже там, где ещё читается в кузове.
pub fn push_shadow(builder: &mut MeshBuilder, car: &Car, offset: Vec2, detail: CarDetail) {
    let profile = car.shape.profile();
    match detail {
        CarDetail::Block => {
            builder.push_quad(car.block(&profile, offset), SHADOW_COLOR.to_linear())
        }
        CarDetail::Full | CarDetail::Silhouette => {
            builder.push_convex(&car.outline(&profile, offset), SHADOW_COLOR.to_linear())
        }
    }
}

/// Кузов со всем, что на нём видно на этой ступени подробности.
///
/// Порядок вызовов — порядок отрисовки: внутри одного слитого меша глубина
/// у всех треугольников одна, и поверх ложится то, что положено позже.
pub fn push_body(builder: &mut MeshBuilder, car: &Car, detail: CarDetail) {
    let profile = car.shape.profile();
    let color = car.color.to_linear();
    if detail == CarDetail::Block {
        builder.push_quad(car.block(&profile, Vec2::ZERO), color);
        return;
    }
    builder.push_convex(&car.outline(&profile, Vec2::ZERO), color);
    if detail == CarDetail::Silhouette {
        return;
    }

    let glass = |from: f32, from_half: f32, to: f32, to_half: f32| {
        [
            car.point(&profile, Vec2::ZERO, from, -from_half),
            car.point(&profile, Vec2::ZERO, from, from_half),
            car.point(&profile, Vec2::ZERO, to, to_half),
            car.point(&profile, Vec2::ZERO, to, -to_half),
        ]
    };
    builder.push_quad(
        glass(
            profile.windshield,
            CABIN_WIDE,
            profile.roof_front,
            CABIN_NARROW,
        ),
        GLASS_FRONT.to_linear(),
    );
    builder.push_quad(
        glass(
            profile.roof_front,
            CABIN_NARROW,
            profile.roof_back,
            CABIN_NARROW,
        ),
        lighten(car.color, ROOF_LIGHTEN).to_linear(),
    );
    builder.push_quad(
        glass(
            profile.roof_back,
            CABIN_NARROW,
            profile.backlight,
            CABIN_WIDE,
        ),
        GLASS_BACK.to_linear(),
    );

    let mirror = lighten(car.color, -MIRROR_DARKEN).to_linear();
    let front = profile.windshield + MIRROR_LONG;
    let back = profile.windshield - MIRROR_LONG;
    for side in [-1.0, 1.0] {
        builder.push_quad(
            [
                car.point(&profile, Vec2::ZERO, back, side * 0.98),
                car.point(&profile, Vec2::ZERO, front, side * 0.98),
                car.point(&profile, Vec2::ZERO, front, side * MIRROR_REACH),
                car.point(&profile, Vec2::ZERO, back, side * MIRROR_REACH),
            ],
            mirror,
        );
    }
}

/// Осветлить (`amount > 0`) или затемнить цвет кузова, в sRGB — как это
/// делают тона стен и скатов в `map::buildings`.
fn lighten(color: Color, amount: f32) -> Color {
    let target = if amount > 0.0 {
        Color::WHITE
    } else {
        Color::BLACK
    };
    color.mix(&target, amount.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Кузов вписан в свой габарит, а зеркала — единственное, что из него
    /// торчит: слой ставит машины по габариту, и вылезший борт заехал бы на
    /// разметку.
    #[test]
    fn the_body_stays_inside_its_own_footprint() {
        for shape in SHAPES {
            let profile = shape.profile();
            let car = Car {
                at: Vec2::ZERO,
                along: Vec2::X,
                color: Color::WHITE,
                shape,
            };
            for point in car.outline(&profile, Vec2::ZERO) {
                assert!(
                    point.x.abs() <= profile.length / 2.0 + 0.001
                        && point.y.abs() <= profile.width / 2.0 + 0.001,
                    "{shape:?}: {point} вне габарита"
                );
            }
        }
    }

    /// Контур кузова выпуклый — на это опирается `push_convex`, который
    /// кладёт его веером из первой вершины вместо `earcutr`.
    #[test]
    fn the_outline_is_convex() {
        for shape in SHAPES {
            let profile = shape.profile();
            let car = Car {
                at: Vec2::ZERO,
                along: Vec2::X,
                color: Color::WHITE,
                shape,
            };
            let outline = car.outline(&profile, Vec2::ZERO);
            for corner in 0..outline.len() {
                let previous = outline[(corner + outline.len() - 1) % outline.len()];
                let point = outline[corner];
                let next = outline[(corner + 1) % outline.len()];
                // контур обходится против часовой (правый борт от носа к
                // корме, потом левый обратно), так что выпуклость — это
                // неотрицательный поворот в каждой вершине
                let turn = (point - previous).perp_dot(next - point);
                assert!(turn >= 0.0, "{shape:?}: вогнутость в {point}, {turn}");
            }
        }
    }

    /// Разметка кузова идёт от носа к корме и не выходит за него: капот,
    /// лобовое, крыша, заднее стекло, багажник.
    #[test]
    fn the_cabin_runs_from_nose_to_tail_in_order() {
        for shape in SHAPES {
            let profile = shape.profile();
            assert!(
                0.5 > profile.windshield
                    && profile.windshield > profile.roof_front
                    && profile.roof_front > profile.roof_back
                    && profile.roof_back > profile.backlight
                    && profile.backlight > -0.5,
                "{shape:?}: разметка кабины не по порядку"
            );
        }
    }

    /// Ступени подробности стоят вершинами именно в том порядке, в каком их
    /// раздаёт зум: дальше — дешевле, иначе LOD не имеет смысла.
    #[test]
    fn detail_only_ever_costs_more_when_closer() {
        let car = Car {
            at: Vec2::ZERO,
            along: Vec2::X,
            color: Color::WHITE,
            shape: CarShape::Sedan,
        };
        let cost = |detail| {
            let mut builder = MeshBuilder::default();
            push_shadow(&mut builder, &car, Vec2::X, detail);
            push_body(&mut builder, &car, detail);
            builder.vertex_count()
        };
        assert!(cost(CarDetail::Block) < cost(CarDetail::Silhouette));
        assert!(cost(CarDetail::Silhouette) < cost(CarDetail::Full));
    }
}
