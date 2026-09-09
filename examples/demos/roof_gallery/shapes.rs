//! Сетка форм: один и тот же контур под всеми тремя крышами города.
//!
//! Материал кровли решает, чем крыша **покрыта**; форму ей выбирает
//! `roofs.rs` — по назначению дома, по контуру и по посеву. Форм три
//! (`RoofShape`): плоская, двускатная, вальмовая. Витрина ставит их в ряд по
//! одному контуру, и слева от ряда — колонка «как решит игра», то есть то,
//! что на этом контуре нарисует город.
//!
//! **Контуры выбраны по случаям, а не для красоты:**
//!
//! * прямоугольник — эталон: заполнение полное, двускатная встаёт;
//! * почти квадрат — площадку конька вальмы поджимает со всех сторон разом;
//! * Г-образный — тот самый дом, ради которого вальма и появилась: до неё он
//!   оставался плоским среди скатных соседей;
//! * П-образный — то же самое, но заполнение прямоугольника уже близко к
//!   порогу `RECT_FILL_MIN`;
//! * гантель — крупное тело с тонким отростком: вдвиг вальмы считается по
//!   толщине **всего** контура, и в отростке скаты обязаны вывернуться.
//!
//! Числа под каждым домом — не пересказ, а те самые, по которым игра
//! принимает решение: их отдаёт `shape_facts` из `roofs.rs`. Без них
//! «вальма» и «плоская крыша с метровой фаской» на картинке неотличимы.

use bevy::prelude::*;
use qwe::map::buildings::{RoofShape, ShapeFacts, shape_facts};
use qwe::map::osm::{AreaKind, BuildingUse, PolyArea};

/// Порядок колонок. «Как решит игра» первой: остальные три — то, из чего она
/// выбирает.
pub(crate) const COLUMNS: [RoofShape; 4] = [
    RoofShape::Auto,
    RoofShape::Flat,
    RoofShape::Gable,
    RoofShape::Hip,
];

/// Шаг сетки, м. По y — самый высокий контур (22 м), подъём крыши над ним и
/// три строки подписи под ним. По x — заметно больше самого широкого контура
/// (гантель, 38 м): сетка форм стоит над сеткой материалов, а та шире, и
/// лишняя ширина здесь достаётся даром — кадр всё равно ограничен высотой.
/// Заодно подпись под домом не приходится ломать по слогам.
pub(crate) const CELL_PITCH: Vec2 = Vec2::new(68.0, 38.0);

/// Ширина колонки подписей слева от сетки, м.
pub(crate) const ROW_LABEL_WIDTH: f32 = 48.0;

/// Зазор от карниза до подписи и высота самой подписи, м.
///
/// Подпись висит **под своим домом**, а не на постоянном отступе от центра
/// клетки: контуры разной высоты, и от центра подпись высокого контура
/// заезжала на дом следующего ряда.
const CAPTION_DROP: f32 = 1.5;
const CAPTION_HEIGHT: f32 = 9.0;

/// Полувысота самого высокого контура, м. Ниже неё уходит подпись, и по сумме
/// сетка знает свой нижний край.
const HALF_HEIGHT_MAX: f32 = 11.0;

/// На сколько подписи уходят ниже центра нижнего ряда, м.
pub(crate) const BOTTOM_REACH: f32 = HALF_HEIGHT_MAX + CAPTION_DROP + CAPTION_HEIGHT;

/// Насколько верх крыши уходит выше центра клетки, м: полувысота контура плюс
/// подъём (`EXTRUDE_RANGE` начинается с 2.5 м).
pub(crate) const TOP_REACH: f32 = HALF_HEIGHT_MAX + 3.0;

/// Дом сетки форм: контур на своём месте и заказанная ему форма.
pub(crate) struct ShapeCell {
    pub(crate) area: PolyArea,
    pub(crate) shape: RoofShape,
    pub(crate) centre: Vec2,
    /// Полувысота контура — по ней подпись садится под карниз этого дома, а
    /// не под середину клетки.
    half_height: f32,
    /// Номер контура сверху вниз — по нему подпись строки ставится один раз
    /// на ряд.
    pub(crate) row: usize,
    /// Номер колонки слева направо — по нему клетка знает, что она первая в
    /// ряду. Раскладка вольна считать `centre` как угодно, признак от этого
    /// не зависит.
    column: usize,
}

impl ShapeCell {
    /// Куда ставить подпись дома: под его карнизом, якорем за верхний край.
    pub(crate) fn caption_at(&self) -> Vec2 {
        self.centre - Vec2::new(0.0, self.half_height + CAPTION_DROP)
    }

    /// Первая клетка ряда — только она несёт подпись самого ряда.
    pub(crate) fn first_in_row(&self) -> bool {
        self.column == 0
    }
}

/// Контур витрины: как его звать и чем он тут занят.
struct Outline {
    name: &'static str,
    note: &'static str,
    /// Кольцо вокруг начала координат, CCW, — как их отдаёт OSM после сборки.
    ring: Vec<Vec2>,
}

/// Стены домов этой сетки одинаковы у всех: сравнивается форма крыши, и
/// разная высота стен под ней сразу сделала бы сравнение нечестным. Высоту
/// задаёт ручка `Height`, значение по умолчанию — типовой частный дом.
fn house(ring: Vec<Vec2>, height: f32) -> PolyArea {
    PolyArea {
        outer: ring,
        holes: Vec::new(),
        kind: AreaKind::Building,
        // частный дом: только ему игра вообще ставит скатную крышу
        // (`roofs::is_gabled`), и сетка форм существует про него
        building_use: BuildingUse::House,
        height: Some(height),
        entrances: Vec::new(),
    }
}

fn outlines() -> Vec<Outline> {
    vec![
        Outline {
            name: "Прямоугольник",
            note: "эталон: заполнение полное,\nдвускатная встаёт",
            ring: rect(Vec2::new(32.0, 18.0)),
        },
        Outline {
            name: "Почти квадрат",
            note: "площадку конька\nподжимает со всех сторон",
            ring: rect(Vec2::new(21.0, 19.0)),
        },
        Outline {
            name: "Г-образный",
            note: "ради него вальма\nи появилась",
            ring: vec![
                Vec2::new(-16.0, -11.0),
                Vec2::new(16.0, -11.0),
                Vec2::new(16.0, -2.0),
                Vec2::new(-2.0, -2.0),
                Vec2::new(-2.0, 11.0),
                Vec2::new(-16.0, 11.0),
            ],
        },
        Outline {
            name: "П-образный",
            note: "заполнение у самого\nпорога",
            ring: vec![
                Vec2::new(-16.0, -11.0),
                Vec2::new(16.0, -11.0),
                Vec2::new(16.0, 11.0),
                Vec2::new(5.5, 11.0),
                Vec2::new(5.5, -2.0),
                Vec2::new(-5.5, -2.0),
                Vec2::new(-5.5, 11.0),
                Vec2::new(-16.0, 11.0),
            ],
        },
        Outline {
            name: "Гантель",
            note: "вдвиг общий на весь контур,\nа отросток тонкий",
            ring: vec![
                Vec2::new(-19.0, -9.0),
                Vec2::new(-1.0, -9.0),
                Vec2::new(-1.0, -1.1),
                Vec2::new(10.0, -1.1),
                Vec2::new(10.0, -5.5),
                Vec2::new(19.0, -5.5),
                Vec2::new(19.0, 5.5),
                Vec2::new(10.0, 5.5),
                Vec2::new(10.0, 1.1),
                Vec2::new(-1.0, 1.1),
                Vec2::new(-1.0, 9.0),
                Vec2::new(-19.0, 9.0),
            ],
        },
    ]
}

/// Прямоугольник со сторонами `size`, CCW, вокруг начала координат.
fn rect(size: Vec2) -> Vec<Vec2> {
    let half = size / 2.0;
    vec![
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(half.x, half.y),
        Vec2::new(-half.x, half.y),
    ]
}

/// Все дома сетки: контуры сверху вниз, формы слева направо. `origin` —
/// центр левой верхней клетки.
pub(crate) fn cells(origin: Vec2, height: f32) -> Vec<ShapeCell> {
    let mut cells = Vec::new();
    for (row, outline) in outlines().into_iter().enumerate() {
        let half_height = outline
            .ring
            .iter()
            .fold(0.0_f32, |top, point| top.max(point.y.abs()));
        for (column, shape) in COLUMNS.into_iter().enumerate() {
            let centre = origin + Vec2::new(column as f32, -(row as f32)) * CELL_PITCH;
            let ring = outline.ring.iter().map(|point| *point + centre).collect();
            cells.push(ShapeCell {
                area: house(ring, height),
                shape,
                centre,
                half_height,
                row,
                column,
            });
        }
    }
    cells
}

/// Сколько в сетке рядов — витрине нужно для кадрирования.
pub(crate) fn row_count() -> usize {
    outlines().len()
}

/// Подпись ряда: имя контура, чем он занят и числа, по которым игра выбирает
/// форму. Заполнение прямоугольника отвечает за двускатную, вдвиг ската — за
/// вальмовую.
pub(crate) fn row_caption(row: usize) -> String {
    let outlines = outlines();
    let outline = &outlines[row];
    let ring = outline.ring.clone();
    let facts = shape_facts(&house(ring, 1.0));
    let numbers = match facts {
        Some(ShapeFacts {
            rect_fill,
            hip_inset,
            ..
        }) => format!(
            "заполнение {rect_fill:.2} · вдвиг вальмы {}",
            match hip_inset {
                Some(inset) => format!("{inset:.2} м"),
                None => "не встаёт".to_string(),
            }
        ),
        None => "вырожденный контур".to_string(),
    };
    format!("{}\n{}\n{numbers}", outline.name, outline.note)
}

/// Подпись дома: что просили, что получилось и на сколько поднялся конёк.
///
/// Отказ печатается прямо: игра не строит двускатную на Г-образном контуре, и
/// подменять его вальмой — значит показывать не то, что нарисует город.
pub(crate) fn cell_caption(cell: &ShapeCell, drawn: RoofShape) -> String {
    let facts = shape_facts(&cell.area);
    let rise = match drawn {
        RoofShape::Gable => facts.as_ref().map(|facts| facts.gable_rise),
        RoofShape::Hip => facts.as_ref().and_then(|facts| facts.hip_rise),
        _ => None,
    };
    let mut lines = match (cell.shape, drawn) {
        (RoofShape::Auto, _) => format!("как решит игра:\n{}", name(drawn)),
        (asked, got) if asked == got => name(got).to_string(),
        (asked, _) => format!("{}:\nотказ, легла плоская", name(asked)),
    };
    if let Some(rise) = rise {
        lines.push_str(&format!("\nконёк +{rise:.2} м"));
    }
    lines
}

fn name(shape: RoofShape) -> &'static str {
    match shape {
        RoofShape::Auto => "как решит игра",
        RoofShape::Flat => "плоская",
        RoofShape::Gable => "двускатная",
        RoofShape::Hip => "вальмовая",
    }
}
