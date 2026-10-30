//! Сборка слитых 2D-мешей слоёв карты: тысячи полигонов OSM в один
//! `Mesh2d` с вершинными цветами (стоковый `ColorMaterial` их умножает;
//! `map::surface::SurfaceMaterial` — умножает и кладёт поверх фактуру).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexAttribute, VertexFormat};
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

#[cfg(test)]
use std::f32::consts::PI;

mod geometry;
mod paint;
mod ribbon;

#[cfg(test)]
pub(crate) use self::geometry::distance_to_path;
#[cfg(test)]
use self::geometry::{ARC_TOLERANCE, MAX_ARC_STEPS};
pub(crate) use self::geometry::{arc_steps, equivalent_width, ring_perimeter};
pub use self::geometry::{min_area_rect, sweep_convex};
#[cfg(test)]
use self::ribbon::FAR_FROM_BREAKS;
pub(crate) use self::ribbon::butt_direction;
pub use self::ribbon::{
    RibbonShape, break_distances, break_profile, merge_close_points, miter_offsets,
    ribbon_merge_distance, ribbon_vertices, to_break_beyond,
};

/// Локальные координаты ленты для шейдера поверхностей (`map::surface`), в
/// метрах. У ленты без полос — `[поперёк, до разрыва, полуширина, 0]`: по ним
/// вода кладёт отмель. У проезжей части с раскладкой полос ([`LaneFrame`]) —
/// `[поперёк от узла сетки полос, до разрыва, нижняя граница, верхняя
/// граница]`, всё от того же узла: колея шейдера лежит на той же сетке, что и
/// линии слоя краски (`roads/paint.rs`). «До разрыва» — знаковое расстояние до
/// края ближайшего разрыва разметки ([`RibbonBreaks`]): по нему гаснет колея у
/// перекрёстка. У полигонов и прочей не-ленточной геометрии — нули. Атрибут
/// есть только у мешей, собранных через [`MeshBuilder::with_surface_coords`]:
/// зданиям, кронам и оверлеям он ни к чему, а это 16 байт на вершину.
///
/// Слой краски несёт тот же атрибут со своим смыслом —
/// `[поперёк от линии, по длине улицы, до разрыва, вид линии]`
/// ([`MeshBuilder::push_paint_strip`]).
///
/// Идентификатор — «высокий случайный», как велит документация
/// `MeshVertexAttribute`: он задаёт порядок атрибутов и не должен совпасть со
/// встроенными.
pub const ATTRIBUTE_RIBBON: MeshVertexAttribute =
    MeshVertexAttribute::new("Ribbon", 2_078_446_317, VertexFormat::Float32x4);

/// Координаты не-ленточной вершины.
const NO_RIBBON: [f32; 4] = [0.0; 4];

/// Рамка вершины для шейдера зданий (`map::buildings::material`): последние два
/// числа всегда `[код материала, посев]`, а первые два значат разное у кровли и
/// у стены — код и говорит, как их читать.
///
/// * **Кровля** — `[ось x, ось y, …]`, длинная ось дома. Фактура считается по
///   мировой координате, повёрнутой в эту ось, а не по развёртке, поэтому
///   координаты вершине не нужны, нужна только ось, и она одинакова у всех
///   вершин дома.
/// * **Стена и гаражная лента** — номер ячейки по одной оси и по другой в их
///   **собственной** раме ([`WallFrame`]), и вот они у каждой вершины свои. У
///   стены это номер панели вдоль основания и этаж вверх по подъёму, у ленты —
///   номер бокса вдоль неё и номер ряда поперёк. Глобальной сеткой ни то, ни
///   другое не покрыть: сетка, не знающая, где кончается стена или лента,
///   режет у края панель, балкон и бокс пополам.
///
/// Отсюда и хранение: не аргумент каждого `push_*`, а состояние сборщика
/// ([`Self::set_roof`] / [`Self::set_wall`]), как код разметки у лент.
///
/// Идентификатор — «высокий случайный», как и у [`ATTRIBUTE_RIBBON`].
pub const ATTRIBUTE_ROOF: MeshVertexAttribute =
    MeshVertexAttribute::new("Roof", 1_704_552_913, VertexFormat::Float32x4);

/// Координата вершины внутри своей кроны единичного радиуса — для шейдера крон
/// (`map::trees::canopy`), когда кроны слиты в один меш (дальние ступени зума,
/// `trees::CrownDetail::Merged`). У кроны-сущности эта координата и есть позиция
/// вершины в её собственном меше; в слитом меше позиция уже мировая, и
/// освещению «шара» нужна координата отдельно.
///
/// Есть только у мешей, собранных через [`MeshBuilder::with_crown_coords`].
/// Идентификатор — «высокий случайный», как и у [`ATTRIBUTE_RIBBON`].
pub const ATTRIBUTE_CROWN: MeshVertexAttribute =
    MeshVertexAttribute::new("Crown", 1_391_207_551, VertexFormat::Float32x2);

/// Вершина без фактуры — кайма, оборудование кровли. Нулевой код материала
/// гасит фактуру, и нулевые первые два числа тогда ни на что не влияют. Ни
/// стена, ни фронтон сюда больше не входят: у обоих код из [`WallKind`].
///
/// [`WallKind`]: crate::map::buildings::material::WallKind
const NO_ROOF: [f32; 4] = [0.0; 4];

/// Оборудование кровли: фактуры нет, как у [`NO_ROOF`] (фрагмент читает слот
/// материала через `max(…, 0)`, и минус там — тот же ноль), а **знак** слота —
/// метка для вершинного шейдера: на дальних ступенях зума он схлопывает такие
/// треугольники в точку (`RoofParams::clutter`). Так оборудование снимается с
/// карты без пересборки слоя зданий и без второго меша — и остаётся в том же
/// painter's порядке, что и дома.
const CLUTTER: [f32; 4] = [0.0, 0.0, -1.0, 0.0];

/// Шаг упаковки в слоте материала: код лежит в остатке от деления на него, а
/// частное — это **число этажей** стены (у кровли и у каймы ноль).
///
/// Зачем этажи вообще едут в шейдер: без них он не знает, где у стены **верх**,
/// а верх — это не мелочь оформления. Стена кончается карнизом, и пока
/// шейдеру был известен только низ (`storey < 0.5`), верхнее окно упиралось в
/// кровлю без единого сантиметра стены над собой; на снимке так не бывает
/// никогда, и первым в глаза бросается именно это.
///
/// Упаковка, а не пятое число атрибута: слот кода — единственное поле, где
/// есть свободный разряд, а лишний `f32` стоил бы четыре байта на каждой
/// вершине слоя зданий. Этажей у самого высокого дома OSM (600 м) двести, то
/// есть `32 · 200 + 16`, всё ещё целое в `f32` без потерь. Шаг был 16 и
/// поднят до 32 вместе с храмовой облицовкой: кодов стало **шестнадцать**
/// (шесть кровель, две гаражные ленты, семь облицовок и дверь), и дверь легла
/// бы ровно в разряд этажей. Следующий подъём — на тридцать втором коде,
/// вместе с зеркалом в `roof.wesl`.
pub const STOREY_STRIDE: u32 = 32;

/// Запас **сверх этажей**, в долях этажа: над последним этажом у дома есть
/// перекрытие, плита кровли и парапет, и вместе это около метра.
///
/// Он не рисуется, а **занимает место**, и в этом вся разница. Первая попытка
/// поправить «окна впритык к кровле» карниз просто закрашивала: полоса ложилась
/// внутрь тех же `1.0 - WINDOW_HIGH`, зазор от этого не менялся ни на пиксель, и
/// сверху лишь появлялась лишняя линия. Стена высотой ровно `этажи × 3 м` этого
/// запаса не имеет физически, поэтому его добавляет **рама**: координата этажа
/// растёт до `storeys + PARAPET_CELLS`, а не до `storeys`.
///
/// Целость счёта от этого не страдает — она про то, что **границы этажей**
/// целые, а не про то, где кончается стена: последний этаж по-прежнему
/// заканчивается на целом числе, и на углу дома обе стены заканчиваются
/// одинаково. Платой идут этажи, нарисованные короче на `1/(этажи + 0.15)`:
/// полтора процента у девятиэтажки, у двухэтажного дома заметнее, но там и
/// полметра под карнизом своё место стоят.
pub const PARAPET_CELLS: f32 = 0.15;

/// Разобрать слот материала обратно — код и число этажей. Зеркало
/// `roof.wesl`: там ровно те же две операции, и других мест, где этот слот
/// толкуют, быть не должно.
///
/// Только для тестов, и это не упущение: в самой игре слот **пишут**, а читает
/// его шейдер. Рядом с ним живёт `roof_coords_for_test` — тот же случай.
#[cfg(test)]
pub fn unpack_material(slot: f32) -> (u32, f32) {
    let packed = slot.max(0.0).round() as u32;
    (packed % STOREY_STRIDE, (packed / STOREY_STRIDE) as f32)
}

/// Рамка кровли одного дома: длинная ось его контура, код материала
/// (`buildings::material::RoofKind::code`) и посев вариаций. `meshing` не
/// знает, что стоит за кодом, — он несёт четыре числа до шейдера.
#[derive(Clone, Copy, Debug)]
pub struct Roof {
    pub axis: Vec2,
    pub material: u32,
    pub seed: f32,
}

impl Roof {
    fn encode(self) -> [f32; 4] {
        [self.axis.x, self.axis.y, self.material as f32, self.seed]
    }
}

/// Рама одной стены: как перевести мировую точку в её собственные
/// координаты — «номер панели» вдоль основания и «этаж» вверх по подъёму.
///
/// Стена в 2.5D — параллелограмм: основание `a→b` и боковые рёбра по вектору
/// подъёма. Обе координаты линейны по точке, поэтому считаются одним
/// скалярным произведением каждая: в полях лежат строки обратной матрицы
/// базиса `[основание, подъём]`, уже умноженные на число панелей и этажей.
///
/// Счёт **в ячейках, а не в метрах**, и это главное в конструкции: число
/// панелей и этажей целое, поэтому у стены не бывает обрезанной панели или
/// полуэтажа под карнизом, а шейдеру не нужно знать ни ширину панели, ни
/// высоту этажа, ни косину подъёма — только `fract`.
///
/// Тем же счётом живёт **гаражный прогон** ([`Self::run`]): ячейка у него —
/// бокс вдоль ленты на ряд поперёк, и огрызка бокса на её торце не бывает
/// ровно потому же, почему у стены не бывает обрезанной панели.
#[derive(Clone, Copy, Debug)]
pub struct WallFrame {
    origin: Vec2,
    to_column: Vec2,
    to_storey: Vec2,
    material: u32,
    seed: f32,
    mark: WallMark,
}

/// Что это за поверхность, если смотреть на неё как на стену. Различать их
/// шейдеру надо, а пятого числа в атрибуте нет — метка едет **посевом**
/// ([`WallFrame::encoded_seed`]), потому что координаты обязаны остаться теми
/// же: швы должны идти через карниз насквозь.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WallMark {
    /// Обычная стена жилого дома: швы, окна и столбцы балконов.
    #[default]
    Balconies,
    /// Стена без балконов — но с окнами: узкий простенок, невысокий дом,
    /// материал, которому балкон не полагается (решает `buildings::layers`).
    Blank,
    /// Стена без проёмов — рисунок материала продолжается, окна и балконы нет.
    /// Трое таких: **фронтон** над карнизом (окно на треугольнике пришлось бы
    /// по скату, и от него осталась бы половина — ради этого продолжения
    /// фронтон и берёт раму стены под ним), **заплата под дверью**
    /// (`buildings::layers::push_doors`), которой ячейка стены отдаётся входу
    /// целиком, чтобы окно не выглядывало из-за дверного полотна, и **заплата
    /// вокруг арочного проёма** (`buildings::arches::push_wall_with_openings`)
    /// — та же конструкция, что дверная, и та же общая заготовка
    /// (`layers::wall_cells`): клетки, задетые вырезом не целиком, отдаются ему
    /// целиком, иначе край проёма резал бы ряд окон пополам.
    Solid,
}

impl WallMark {
    /// Метка, прочитанная из посева, — разбор, обратный
    /// [`WallFrame::encoded_seed`] и зеркальный шейдерному
    /// `roof.wesl::wall_shade` (там ветка `Solid` зовётся `gable`). Пороги
    /// лежат посередине зазоров между диапазонами, и записаны они **здесь
    /// одни на весь крейт**: каждый, кто разбирает посев по-своему, рано или
    /// поздно разойдётся с этой кодировкой.
    pub fn of_seed(seed: f32) -> Self {
        if seed < -2.5 {
            Self::Solid
        } else if seed < 0.0 {
            Self::Blank
        } else {
            Self::Balconies
        }
    }
}

impl WallFrame {
    /// `None` — вырожденная рама: стена нулевой длины или идущая ровно по
    /// подъёму (её `silhouette_edges` и не отдаёт, но базис на ней необратим).
    pub fn new(
        a: Vec2,
        b: Vec2,
        lift: Vec2,
        columns: f32,
        storeys: f32,
        material: u32,
        seed: f32,
    ) -> Option<Self> {
        Self::framed(
            a,
            b,
            lift,
            columns,
            // Верх стены — это `storeys + PARAPET_CELLS`, а не `storeys`: над
            // последним этажом лежит запас под карниз, и он настоящий, а не
            // нарисованный. Всё, что выше целого числа этажей, шейдер и считает
            // карнизом — отдельной константы ему для этого не нужно.
            storeys + PARAPET_CELLS,
            // Число этажей уходит в шейдер **вместе с кодом** ([`STOREY_STRIDE`]):
            // по нему он и находит границу, выше которой начинается карниз.
            material + STOREY_STRIDE * storeys.max(0.0) as u32,
            seed,
        )
    }

    /// Рама **одного проёма**: четырёхугольник `a→b` шириной в полотно и
    /// `up` высотой в него же ложится ровно в клетку `[0, 1]²`.
    ///
    /// Этажей в ней нет, и запаса под карниз тоже: проём — не стена, у него
    /// своя геометрия и свой код материала, а шейдеру от рамы нужно только
    /// «где я внутри полотна». Так дверь встаёт **точно там, где её поставил
    /// генератор входов**, а не в ближайшей панели стены: панельная сетка о
    /// дверях ничего не знает, и снести дверь к центру ячейки значило бы
    /// разъехаться с гизмой дверей и с пешеходом, который к ней идёт.
    pub fn opening(a: Vec2, b: Vec2, up: Vec2, material: u32, seed: f32) -> Option<Self> {
        Self::framed(a, b, up, 1.0, 1.0, material, seed)
    }

    /// Рама **прогона**: прямоугольная грань с началом в углу, единичной осью
    /// `along` вдоль неё и шагом ячейки в метрах по каждой из осей. Такова
    /// гаражная лента ([`buildings::garages`]): её ячейка — бокс вдоль
    /// прогона на ряд поперёк.
    ///
    /// Базис тут ортонормирован, поэтому число клеток по оси — это единица,
    /// делённая на шаг, и обращать нечего: то же тело, что у стены, сводится
    /// к делению на шаг. Этажей в раме нет, и в слот материала они не
    /// упакованы: у кровли, чем бы она ни была крыта, их ноль.
    ///
    /// [`buildings::garages`]: crate::map::buildings::garages
    pub fn run(
        origin: Vec2,
        along: Vec2,
        bay: f32,
        row: f32,
        material: u32,
        seed: f32,
    ) -> Option<Self> {
        let along = along.try_normalize()?;
        if bay <= 0.0 || row <= 0.0 {
            return None;
        }
        let across = Vec2::new(-along.y, along.x);
        Self::framed(
            origin,
            origin + along,
            across,
            1.0 / bay,
            1.0 / row,
            material,
            seed,
        )
    }

    /// Общее тело всех трёх: строки обратной матрицы базиса
    /// `[основание, подъём]`, умноженные на число клеток по каждой оси.
    fn framed(
        a: Vec2,
        b: Vec2,
        lift: Vec2,
        columns: f32,
        rows: f32,
        material: u32,
        seed: f32,
    ) -> Option<Self> {
        let along = b - a;
        let det = along.perp_dot(lift);
        if det.abs() < 1e-6 {
            return None;
        }
        Some(Self {
            origin: a,
            to_column: Vec2::new(lift.y, -lift.x) / det * columns,
            to_storey: Vec2::new(-along.y, along.x) / det * rows,
            material,
            seed,
            mark: WallMark::Balconies,
        })
    }

    /// Та же рама с другой меткой. Метка **заменяется**, а не накладывается:
    /// фронтон метит себя сам поверх стены, которая на глухом доме уже
    /// помечена, и повторный вызов обязан ничего не портить. Прежняя версия
    /// хранила метку знаком посева и переворачивала его на месте — второе
    /// отрицание возвращало балконы обратно, и от этого приходилось
    /// защищаться проверкой внутри.
    pub fn marked(self, mark: WallMark) -> Self {
        Self { mark, ..self }
    }

    /// Посев вместе с меткой в одном числе: `[0, 1)` — обычная стена,
    /// `(-2, -1]` — без балконов, `(-4, -3]` — стена без проёмов. Зеркало
    /// разбора лежит в `roof.wesl::wall_shade`, а на стороне Rust —
    /// [`WallMark::of_seed`]; диапазоны разведены с запасом, чтобы порог между
    /// ними не зависел от точности числа.
    fn encoded_seed(self) -> f32 {
        match self.mark {
            WallMark::Balconies => self.seed,
            WallMark::Blank => -self.seed - 1.0,
            WallMark::Solid => -self.seed - 3.0,
        }
    }

    fn encode_at(self, point: Vec2) -> [f32; 4] {
        let offset = point - self.origin;
        [
            offset.dot(self.to_column),
            offset.dot(self.to_storey),
            self.material as f32,
            self.encoded_seed(),
        ]
    }
}

/// Чем сборщик заполняет [`ATTRIBUTE_ROOF`] у вершин, которые лягут дальше.
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// Одно значение на все вершины: кровля, кайма, оборудование.
    Same([f32; 4]),
    /// Своё значение у каждой вершины — стена с её фронтоном и гаражная лента.
    Wall(WallFrame),
}

impl Default for Frame {
    fn default() -> Self {
        Self::Same(NO_ROOF)
    }
}

impl Frame {
    fn at(self, point: Vec2) -> [f32; 4] {
        match self {
            Self::Same(frame) => frame,
            Self::Wall(wall) => wall.encode_at(point),
        }
    }
}

/// Раскладка полос проезжей части поперёк ленты, м от её осевой (плюс —
/// влево по ходу пути): узел сетки полос `origin` и границы проезжей части
/// `low..high`. Границы полос лежат в `origin + k · шаг полосы`; шаг один на
/// город, его знает шейдер. Одна раскладка на колею шейдера поверхностей и на
/// линии слоя краски (`roads/paint.rs`), поэтому на клине, где раскладка
/// плывёт от узкого сечения к широкому, колея идёт ровно между линиями.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LaneFrame {
    pub origin: f32,
    pub low: f32,
    pub high: f32,
}

impl LaneFrame {
    /// Раскладка на доле `t` пути от `self` к `to` — по ней клин переходит от
    /// узкого сечения к широкому.
    pub fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            origin: self.origin + (to.origin - self.origin) * t,
            low: self.low + (to.low - self.low) * t,
            high: self.high + (to.high - self.high) * t,
        }
    }

    /// Координаты вершины ленты в этой раскладке ([`ATTRIBUTE_RIBBON`]).
    fn coords(self, across: f32, to_break: f32) -> [f32; 4] {
        [
            across - self.origin,
            to_break,
            self.low - self.origin,
            self.high - self.origin,
        ]
    }
}

/// Вершина линии слоя краски ([`MeshBuilder::push_paint_strip`]): длина улицы
/// в этой точке (по ней идут штрихи, и фаза не рвётся на шве ways), «до
/// разрыва» и видимость — ноль там, где линия рождается из клина.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PaintStation {
    pub along: f32,
    pub to_break: f32,
    pub alpha: f32,
}

/// Разрыв разметки: точка на дороге (перекрёсток, тупик) и полудлина разрыва
/// вдоль неё, м. Точка — мировая: лента проецирует её на свой путь сама,
/// поэтому узел OSM и сглаженная осевая, по которой лента построена, могут
/// расходиться.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Break {
    pub at: Vec2,
    pub reach: f32,
}

/// Где у ленты разрывы разметки.
#[derive(Clone, Copy)]
pub enum RibbonBreaks<'a> {
    /// Торцы ленты, и только они: «до разрыва» — до ближайшего торца.
    Ends,
    /// Заданный список. Торец, которого в списке нет, — стык с продолжением
    /// той же дороги: разметка идёт сквозь него.
    At(&'a [Break]),
}

/// Стык сегментов ленты на изломе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonJoin {
    /// Сведение по биссектрисе с ограничением `MITER_LIMIT`
    /// (`meshing::ribbon`).
    Miter,
    /// Дуга радиуса в полуширину на внешней стороне поворота — то же, что
    /// `stroke-linejoin: round` у Mapnik, которым нарисован osm-carto.
    Round,
}

/// Торец разомкнутой ленты.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonCap {
    /// Срез ровно по последней точке пути.
    Butt,
    /// Полудиск радиуса в полуширину за последней точкой. Торцы двух дорог в
    /// общем узле перекрываются и сливаются в скруглённый стык — так узлы
    /// выглядят в OSM, где это `stroke-linecap: round`.
    Round,
}

#[derive(Clone, Default)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    skipped_polygons: usize,
    /// `Some` — меш собирается для `SurfaceMaterial` и несёт
    /// [`ATTRIBUTE_RIBBON`] на каждой вершине.
    ribbon: Option<Vec<[f32; 4]>>,
    /// Раскладка полос для лент, которые лягут дальше — `[у начала, у конца]`
    /// ([`Self::set_lanes`], [`Self::set_lane_taper`]); у ленты постоянной
    /// ширины обе одинаковы, у клина ([`Self::push_taper`]) плывут. `None` —
    /// лента без полос.
    lanes: [Option<LaneFrame>; 2],
    /// Раскладка, плывущая вдоль ленты ([`Self::set_lane_profile`]): пары
    /// «метров от начала пути — раскладка» по возрастанию; пусто — вся лента
    /// в `lanes[0]`.
    lane_profile: Vec<(f32, LaneFrame)>,
    /// `Some` — меш собирается для `buildings::material::RoofMaterial` и несёт
    /// [`ATTRIBUTE_ROOF`] на каждой вершине.
    roof: Option<Vec<[f32; 4]>>,
    /// Чем заполняется [`ATTRIBUTE_ROOF`] у геометрии, которая ляжет дальше
    /// ([`Self::set_roof`] / [`Self::set_wall`]); нули — без фактуры.
    frame: Frame,
    /// `Some` — меш собирается для слитых крон и несёт [`ATTRIBUTE_CROWN`] на
    /// каждой вершине.
    crown: Option<Vec<[f32; 2]>>,
}

impl MeshBuilder {
    /// Сборщик с локальными координатами лент ([`ATTRIBUTE_RIBBON`]) — для
    /// слоёв, которые рисует `map::surface::SurfaceMaterial`; без них тот
    /// материал меш не примет.
    pub fn with_surface_coords() -> Self {
        Self {
            ribbon: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// Сборщик с рамками кровли ([`ATTRIBUTE_ROOF`]) — для зданиевых слоёв,
    /// которые рисует `map::buildings::material::RoofMaterial`; без атрибута
    /// тот материал меш не примет.
    pub fn with_roof_coords() -> Self {
        Self {
            roof: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// Сборщик с координатами кроны ([`ATTRIBUTE_CROWN`]). Геометрия,
    /// положенная в него напрямую, считается нарисованной в пространстве
    /// кроны единичного радиуса — её координата кроны равна позиции: так
    /// строится шаблон дальней кроны. Слитый меш набирается из шаблонов
    /// через [`Self::push_crown`].
    pub fn with_crown_coords() -> Self {
        Self {
            crown: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// Раскладка полос лент, положенных после этого вызова: колея шейдера
    /// улиц лежит только по ней, и проезд без полос её не получает. Без
    /// координат поверхности раскладку некуда записать.
    pub fn set_lanes(&mut self, lanes: Option<LaneFrame>) {
        self.lanes = [lanes; 2];
        self.lane_profile.clear();
    }

    /// Раскладка клина ([`Self::push_taper`]), плывущая от `from` у его
    /// начала к `to` у конца.
    pub fn set_lane_taper(&mut self, from: Option<LaneFrame>, to: Option<LaneFrame>) {
        self.lanes = [from, to];
        self.lane_profile.clear();
    }

    /// Раскладка ленты [`Self::push_ribbon_shaped`], заданная по длине пути:
    /// `profile` — пары «метров от начала — раскладка» по возрастанию, между
    /// ними — по прямой, за крайними — крайние. Так колея половины идёт за
    /// линиями краски через клин слияния (`roads/merges.rs`): вершина ленты
    /// встаёт на каждую пару. Пустой профиль — вся лента в `body`.
    pub fn set_lane_profile(&mut self, body: Option<LaneFrame>, profile: Vec<(f32, LaneFrame)>) {
        self.lanes = [body; 2];
        self.lane_profile = profile;
    }

    /// Раскладка в `along` метрах от начала пути ленты.
    fn lanes_at(&self, along: f32) -> Option<LaneFrame> {
        let profile = &self.lane_profile;
        let (Some(first), Some(last)) = (profile.first(), profile.last()) else {
            return self.lanes[0];
        };
        if along <= first.0 {
            return Some(first.1);
        }
        if along >= last.0 {
            return Some(last.1);
        }
        let index = profile.partition_point(|&(at, _)| at <= along);
        let (from, to) = (profile[index - 1], profile[index]);
        let span = to.0 - from.0;
        let t = if span > 0.0 {
            (along - from.0) / span
        } else {
            1.0
        };
        Some(from.1.lerp(to.1, t))
    }

    /// Кровля, которой принадлежит геометрия после этого вызова; `None` —
    /// кайма, оборудование: всё, чему фактуры не полагается вовсе. Стена и
    /// гаражная лента сюда не ходят, у них [`Self::set_wall`].
    pub fn set_roof(&mut self, roof: Option<Roof>) {
        self.frame = Frame::Same(roof.map_or(NO_ROOF, Roof::encode));
    }

    /// Геометрия после этого вызова — оборудование кровли ([`CLUTTER`]): без
    /// фактуры, и дальние ступени зума прячут её в шейдере.
    pub fn set_clutter(&mut self) {
        self.frame = Frame::Same(CLUTTER);
    }

    /// Грань со своими ячейками (стена, гаражная лента), которой принадлежит
    /// геометрия после этого вызова: в отличие от кровли, рамка тут считается
    /// **у каждой вершины** по её месту в грани. `None` — рама вырождена,
    /// геометрия уходит без фактуры.
    pub fn set_wall(&mut self, wall: Option<WallFrame>) {
        self.frame = match wall {
            Some(wall) => Frame::Wall(wall),
            None => Frame::Same(NO_ROOF),
        };
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Накопленные треугольники в плоскости карты — тому, кто спрашивает, что
    /// слой уже кроет (`roads::paint::PaintGround`: асфальт под зеброй и
    /// стоп-линией).
    pub fn triangles(&self) -> impl Iterator<Item = [Vec2; 3]> + '_ {
        self.indices.chunks_exact(3).map(|triangle| {
            [0, 1, 2].map(|at| {
                let [x, y, _] = self.positions[triangle[at] as usize];
                Vec2::new(x, y)
            })
        })
    }

    /// Ближайшие к точке вершины — не дальше `radius`, ближними первыми, не
    /// больше `limit`; первое поле пары — **квадрат** расстояния (`radius`
    /// линейный, вызов `dev.rs::on_probe_layers` берёт корень). Пробе слоёв:
    /// вершины у точки говорят, где край слоя, когда треугольники точку уже
    /// не накрывают. Координаты слоя — мировые (адаптер сажает его с
    /// тождественным xy), так что точки сравниваются напрямую.
    pub fn nearest_vertices(&self, at: Vec2, radius: f32, limit: usize) -> Vec<(f32, Vec2)> {
        let mut hits: Vec<(f32, Vec2)> = self
            .positions
            .iter()
            .filter_map(|&[x, y, _]| {
                let point = Vec2::new(x, y);
                let d2 = point.distance_squared(at);
                (d2 <= radius * radius).then_some((d2, point))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.truncate(limit);
        hits
    }

    /// Координаты ленты — тест проверяет по ним, что легло в атрибут.
    #[cfg(test)]
    pub fn ribbon_coords_for_test(&self) -> Option<&[[f32; 4]]> {
        self.ribbon.as_deref()
    }

    /// Координаты кроны — тест проверяет по ним, что легло в атрибут.
    #[cfg(test)]
    pub fn crown_coords_for_test(&self) -> Option<&[[f32; 2]]> {
        self.crown.as_deref()
    }

    /// Рамки кровли — тест проверяет по ним, что легло в атрибут.
    #[cfg(test)]
    pub fn roof_coords_for_test(&self) -> Option<&[[f32; 4]]> {
        self.roof.as_deref()
    }

    fn push_vertex(&mut self, position: Vec2, rgba: [f32; 4], ribbon: [f32; 4]) {
        self.positions.push([position.x, position.y, 0.0]);
        self.colors.push(rgba);
        if let Some(coords) = &mut self.ribbon {
            coords.push(ribbon);
        }
        if let Some(frames) = &mut self.roof {
            frames.push(self.frame.at(position));
        }
        if let Some(local) = &mut self.crown {
            local.push(position.to_array());
        }
    }

    /// Координаты вершины ленты в текущей раскладке полос.
    fn coords(&self, across: f32, to_break: f32, half_width: f32) -> [f32; 4] {
        Self::coords_in(self.lanes[0], across, to_break, half_width)
    }

    fn coords_in(
        lanes: Option<LaneFrame>,
        across: f32,
        to_break: f32,
        half_width: f32,
    ) -> [f32; 4] {
        match lanes {
            Some(frame) => frame.coords(across, to_break),
            None => [across, to_break, half_width, 0.0],
        }
    }

    pub fn skipped_polygons(&self) -> usize {
        self.skipped_polygons
    }

    /// Сколько вершин уже накоплено — тестам, чтобы сравнивать объём
    /// геометрии, не разбирая готовый меш.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Накопленные вершины — только для тестов, которым надо проверить, куда
    /// именно легла геометрия (высота проёма арки, например).
    #[cfg(test)]
    pub fn positions_for_test(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Кроет ли точку хоть один треугольник — тестам, которым важно, что
    /// место закрыто слоем, а не какими вершинами.
    #[cfg(test)]
    pub fn covers_for_test(&self, point: Vec2) -> bool {
        self.triangles().any(|[a, b, c]| {
            let sides =
                [(a, b), (b, c), (c, a)].map(|(from, to)| (to - from).perp_dot(point - from));
            sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0)
        })
    }

    /// Накопленные цвета — тестам мягких краёв: «кайма гаснет» проверяется
    /// альфой на вершине, а по одним позициям её не отличить от ленты, которая
    /// просто шире.
    #[cfg(test)]
    pub fn colors_for_test(&self) -> &[[f32; 4]] {
        &self.colors
    }

    /// Накопленный атрибут ленты ([`ATTRIBUTE_RIBBON`]) — тестам краски,
    /// которым важен вид линии (код в `w`), а не только где она легла.
    #[cfg(test)]
    pub fn ribbon_for_test(&self) -> &[[f32; 4]] {
        self.ribbon.as_deref().unwrap_or(&[])
    }

    /// Полигон с дырками через earcut. Вырожденный/кривой — пропуск со
    /// счётчиком, один плохой контур OSM не должен ронять всю карту.
    pub fn push_polygon(&mut self, outer: &[Vec2], holes: &[Vec<Vec2>], color: LinearRgba) {
        let holes: Vec<(&[Vec2], LinearRgba)> =
            holes.iter().map(|hole| (hole.as_slice(), color)).collect();
        self.push_polygon_graded((outer, color), &holes);
    }

    /// Полигон с дырками, у которого **каждое кольцо своего цвета**: цвет
    /// вершины — цвет её кольца, и между кольцами GPU тянет линейный градиент.
    /// Так кладётся отмель площадной воды (`map::water`): полоса между
    /// двумя вложенными офсетами берега — внешнее кольцо светлее, дырка
    /// темнее. Треугольник, чьи три вершины легли на одно кольцо, закрашен
    /// ровно — на полосе в полметра это ошибка меньше одной ступени.
    pub fn push_polygon_graded(
        &mut self,
        outer: (&[Vec2], LinearRgba),
        holes: &[(&[Vec2], LinearRgba)],
    ) {
        if outer.0.len() < 3 {
            self.skipped_polygons += 1;
            return;
        }

        let count = outer.0.len() + holes.iter().map(|hole| hole.0.len()).sum::<usize>();
        let mut coordinates: Vec<f64> = Vec::with_capacity(count * 2);
        let mut colors: Vec<[f32; 4]> = Vec::with_capacity(count);
        let mut hole_starts = Vec::with_capacity(holes.len());
        for point in outer.0 {
            coordinates.push(point.x as f64);
            coordinates.push(point.y as f64);
            colors.push(outer.1.to_f32_array());
        }
        for (hole, color) in holes {
            hole_starts.push(coordinates.len() / 2);
            for point in *hole {
                coordinates.push(point.x as f64);
                coordinates.push(point.y as f64);
                colors.push(color.to_f32_array());
            }
        }

        let Ok(triangles) = earcutr::earcut(&coordinates, &hole_starts, 2) else {
            self.skipped_polygons += 1;
            return;
        };
        if triangles.is_empty() {
            self.skipped_polygons += 1;
            return;
        }

        let base = self.positions.len() as u32;
        for (chunk, rgba) in coordinates.chunks_exact(2).zip(colors) {
            self.push_vertex(Vec2::new(chunk[0] as f32, chunk[1] as f32), rgba, NO_RIBBON);
        }
        self.indices
            .extend(triangles.into_iter().map(|index| base + index as u32));
    }

    /// Уже собранная геометрия, приложенная со сдвигом и масштабом. Силуэт
    /// тени кроны один на вариант и повторяется под тысячами деревьев —
    /// триангулировать его каждый раз заново незачем.
    pub fn push_template(&mut self, template: &MeshBuilder, offset: Vec2, scale: f32) {
        let base = self.positions.len() as u32;
        self.positions
            .extend(template.positions.iter().map(|point| {
                [
                    point[0] * scale + offset.x,
                    point[1] * scale + offset.y,
                    point[2],
                ]
            }));
        self.colors.extend_from_slice(&template.colors);
        if let Some(coords) = &mut self.ribbon {
            match &template.ribbon {
                Some(source) => {
                    coords.extend(source.iter().map(|[across, to_break, half_width, mode]| {
                        [across * scale, to_break * scale, half_width * scale, *mode]
                    }))
                }
                None => coords.extend(std::iter::repeat_n(NO_RIBBON, template.positions.len())),
            }
        }
        if let Some(frames) = &mut self.roof {
            // рамка стены зависит от точки, поэтому считается по уже
            // перенесённой геометрии, а не по образцу
            let frame = self.frame;
            frames.extend(
                self.positions[base as usize..]
                    .iter()
                    .map(|point| frame.at(Vec2::new(point[0], point[1]))),
            );
        }
        if let Some(local) = &mut self.crown {
            // координата кроны — место в образце, до сдвига и масштаба
            local.extend(template.positions.iter().map(|point| [point[0], point[1]]));
        }
        self.indices
            .extend(template.indices.iter().map(|index| base + index));
    }

    /// Шаблон кроны единичного радиуса, приложенный к стволу `at` с радиусом
    /// `radius`, со слотом яркости `brightness`, **запечённым в цвет вершин**:
    /// у кроны-сущности этот множитель лежит в юниформе материала, а у слитого
    /// меша материал один на все кроны. Шейдер умножает цвет вершины на
    /// `brightness` первым же действием, и цвет по треугольнику интерполируется
    /// линейно, так что запекание даёт тот же пиксель.
    ///
    /// Координата кроны ([`ATTRIBUTE_CROWN`]) берётся из образца, если он её
    /// несёт, иначе — его позиция: образец нарисован в пространстве кроны.
    pub fn push_crown(&mut self, template: &MeshBuilder, at: Vec2, radius: f32, brightness: f32) {
        let base = self.positions.len() as u32;
        self.positions.extend(
            template
                .positions
                .iter()
                .map(|point| [point[0] * radius + at.x, point[1] * radius + at.y, point[2]]),
        );
        self.colors.extend(
            template
                .colors
                .iter()
                .map(|&[r, g, b, a]| [r * brightness, g * brightness, b * brightness, a]),
        );
        if let Some(coords) = &mut self.ribbon {
            coords.extend(std::iter::repeat_n(NO_RIBBON, template.positions.len()));
        }
        if let Some(frames) = &mut self.roof {
            frames.extend(std::iter::repeat_n(NO_ROOF, template.positions.len()));
        }
        if let Some(local) = &mut self.crown {
            match &template.crown {
                Some(source) => local.extend_from_slice(source),
                None => local.extend(template.positions.iter().map(|point| [point[0], point[1]])),
            }
        }
        self.indices
            .extend(template.indices.iter().map(|index| base + index));
    }

    /// Сколько индексов уже накоплено — отметка, от которой
    /// [`Self::area_since`] считает площадь.
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    /// Сумма площадей треугольников, положенных после отметки `first_index`
    /// ([`Self::index_count`]). Перекрытия считаются дважды — это площадь
    /// нарисованного, а не покрытого.
    pub fn area_since(&self, first_index: usize) -> f32 {
        let point = |index: u32| {
            let [x, y, _] = self.positions[index as usize];
            Vec2::new(x, y)
        };
        self.indices[first_index..]
            .chunks_exact(3)
            .map(|triangle| {
                let (a, b, c) = (point(triangle[0]), point(triangle[1]), point(triangle[2]));
                (b - a).perp_dot(c - a).abs() / 2.0
            })
            .sum()
    }

    /// Прямоугольник по AABB (для тайловых оверлеев).
    pub fn push_rect(&mut self, min: Vec2, max: Vec2, color: LinearRgba) {
        self.push_quad(
            [
                Vec2::new(min.x, min.y),
                Vec2::new(max.x, min.y),
                Vec2::new(max.x, max.y),
                Vec2::new(min.x, max.y),
            ],
            color,
        );
    }

    pub(crate) fn push_quad(&mut self, corners: [Vec2; 4], color: LinearRgba) {
        self.push_quad_gradient(corners, [color; 4]);
    }

    /// **Выпуклый** контур веером из первой вершины.
    ///
    /// [`Self::push_polygon`] умеет любой контур с дырами и потому зовёт
    /// `earcutr`; на десятке вершин это в разы дороже самой укладки, а
    /// вызывается на каждый кузов машины — двадцать две тысячи раз на город.
    /// Выпуклость здесь — обязанность вызывающего: на невыпуклом контуре веер
    /// заедет за его собственный край.
    pub(crate) fn push_convex(&mut self, outline: &[Vec2], color: LinearRgba) {
        if outline.len() < 3 {
            self.skipped_polygons += 1;
            return;
        }
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        for &point in outline {
            self.push_vertex(point, rgba, NO_RIBBON);
        }
        for index in 1..outline.len() as u32 - 1 {
            self.indices.extend([base, base + index, base + index + 1]);
        }
    }

    /// Замкнутый веер вокруг `center` с цветом на каждую вершину обода — так
    /// рисуется ломтик купола (`buildings::temples`): свет на нём меняется по
    /// окружности, и плоский цвет на ломтик дал бы гранёный купол.
    pub(crate) fn push_fan_gradient(
        &mut self,
        center: Vec2,
        center_color: LinearRgba,
        rim: &[(Vec2, LinearRgba)],
    ) {
        if rim.len() < 3 {
            self.skipped_polygons += 1;
            return;
        }
        let base = self.positions.len() as u32;
        self.push_vertex(center, center_color.to_f32_array(), NO_RIBBON);
        for (point, color) in rim {
            self.push_vertex(*point, color.to_f32_array(), NO_RIBBON);
        }
        let count = rim.len() as u32;
        for index in 0..count {
            self.indices
                .extend([base, base + 1 + index, base + 1 + (index + 1) % count]);
        }
    }

    /// Треугольник одного цвета — грань шатра (`buildings::roofs::TentRoof`).
    pub(crate) fn push_triangle(&mut self, corners: [Vec2; 3], color: LinearRgba) {
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        for corner in corners {
            self.push_vertex(corner, rgba, NO_RIBBON);
        }
        self.indices.extend([base, base + 1, base + 2]);
    }

    /// Квад с цветом на каждую вершину — для вертикального градиента стен
    /// экструдированных зданий.
    pub(crate) fn push_quad_gradient(&mut self, corners: [Vec2; 4], colors: [LinearRgba; 4]) {
        self.push_quad_full(corners, colors, [NO_RIBBON; 4]);
    }

    /// Квад с цветом и координатами ленты на каждую вершину.
    fn push_quad_full(
        &mut self,
        corners: [Vec2; 4],
        colors: [LinearRgba; 4],
        coords: [[f32; 4]; 4],
    ) {
        let base = self.positions.len() as u32;
        for ((corner, color), ribbon) in corners.into_iter().zip(colors).zip(coords) {
            self.push_vertex(corner, color.to_f32_array(), ribbon);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Обрезать уже собранную геометрию прямоугольником `min..max`: каждый
    /// треугольник режется по четырём сторонам, вершины на линии реза получают
    /// цвет и координаты фактуры **линейной интерполяцией** — той же, какой их
    /// по треугольнику ведёт GPU, так что внутри окна картинка не меняется ни
    /// на пиксель.
    ///
    /// Нужна витрине пересечений (`examples/demos/roads`): та ставит окна
    /// разных мест города встык, и всё, что слой рисует за своим окном — хвост
    /// дороги, целый дом на краю вырезки, квад земли во всю карту, — иначе
    /// ложится в окно соседа. Игре она не нужна: у неё карта одна.
    pub fn clip_to_rect(&mut self, min: Vec2, max: Vec2) {
        /// Вершина со всем, что на ней едет: позиция (с z), цвет, две
        /// необязательные четвёрки фактуры и необязательная координата кроны.
        type Vertex = ([f32; 3], [f32; 4], [f32; 4], [f32; 4], [f32; 2]);
        fn mix<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        }

        let vertex = |index: u32| -> Vertex {
            let i = index as usize;
            (
                self.positions[i],
                self.colors[i],
                self.ribbon.as_ref().map_or([0.0; 4], |ribbon| ribbon[i]),
                self.roof.as_ref().map_or([0.0; 4], |roof| roof[i]),
                self.crown.as_ref().map_or([0.0; 2], |crown| crown[i]),
            )
        };
        // сторона окна: ось, граница и с какой её стороны — «внутри»
        let sides = [
            (0, min.x, 1.0),
            (0, max.x, -1.0),
            (1, min.y, 1.0),
            (1, max.y, -1.0),
        ];

        let mut kept: Vec<Vertex> = Vec::new();
        let mut indices = Vec::new();
        for triangle in self.indices.chunks_exact(3) {
            let mut polygon: Vec<Vertex> = triangle.iter().map(|index| vertex(*index)).collect();
            for (axis, bound, sign) in sides {
                let inside = |v: &Vertex| (v.0[axis] - bound) * sign >= 0.0;
                let mut clipped = Vec::with_capacity(polygon.len() + 1);
                for (i, a) in polygon.iter().enumerate() {
                    let b = &polygon[(i + 1) % polygon.len()];
                    if inside(a) {
                        clipped.push(*a);
                    }
                    if inside(a) != inside(b) {
                        let t = (bound - a.0[axis]) / (b.0[axis] - a.0[axis]);
                        clipped.push((
                            mix(a.0, b.0, t),
                            mix(a.1, b.1, t),
                            mix(a.2, b.2, t),
                            mix(a.3, b.3, t),
                            mix(a.4, b.4, t),
                        ));
                    }
                }
                polygon = clipped;
                if polygon.is_empty() {
                    break;
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            // треугольник, обрезанный выпуклым окном, выпукл — веер от первой
            let base = kept.len() as u32;
            kept.extend(&polygon);
            for i in 1..polygon.len() as u32 - 1 {
                indices.extend([base, base + i, base + i + 1]);
            }
        }

        self.positions = kept.iter().map(|vertex| vertex.0).collect();
        self.colors = kept.iter().map(|vertex| vertex.1).collect();
        if let Some(ribbon) = &mut self.ribbon {
            *ribbon = kept.iter().map(|vertex| vertex.2).collect();
        }
        if let Some(roof) = &mut self.roof {
            *roof = kept.iter().map(|vertex| vertex.3).collect();
        }
        if let Some(crown) = &mut self.crown {
            *crown = kept.iter().map(|vertex| vertex.4).collect();
        }
        self.indices = indices;
    }

    /// Склеить куски в один сборщик — ровно в том порядке, в каком их
    /// геометрия легла бы в один сборщик подряд: индексы каждого куска
    /// сдвигаются на вершины кусков перед ним, так что меш выходит тем же байт
    /// в байт. Так собирается слой, чьи куски строятся по потокам
    /// (`cars::mesh_bodies`).
    ///
    /// Копирование идёт **тоже по потокам** — по `groups` подряд идущих
    /// кусков на поток, в заранее выделенные буферы: склейка в один поток
    /// стоила на слое машин Калуги (полтора миллиона вершин, полсотни
    /// мегабайт) столько же, сколько сама сборка кусков, — это копирование и
    /// первое касание свежей памяти, а не арифметика. Буферы выделяются нулями
    /// (`vec![0; n]` берёт у системы уже обнулённые страницы), так что первое
    /// касание достаётся потокам.
    ///
    /// Только куски без фактуры (ни ленты, ни кровли, ни координат кроны):
    /// склеивать их пока нечему, а атрибут, которого нет у одного из кусков,
    /// лёг бы не на каждую вершину.
    pub fn concat(parts: &[MeshBuilder], groups: usize) -> MeshBuilder {
        assert!(
            parts
                .iter()
                .all(|part| part.ribbon.is_none() && part.roof.is_none() && part.crown.is_none()),
            "concat: куски с фактурой не склеиваются"
        );
        let vertices = parts.iter().map(|part| part.positions.len()).sum();
        let index_count = parts.iter().map(|part| part.indices.len()).sum();
        let mut positions = vec![[0.0; 3]; vertices];
        let mut colors = vec![[0.0; 4]; vertices];
        let mut indices = vec![0; index_count];

        /// Куда ляжет один кусок: его доли трёх буферов и сдвиг индексов.
        struct Slot<'a> {
            part: &'a MeshBuilder,
            positions: &'a mut [[f32; 3]],
            colors: &'a mut [[f32; 4]],
            indices: &'a mut [u32],
            offset: u32,
        }
        let mut slots = Vec::with_capacity(parts.len());
        let (mut positions_rest, mut colors_rest, mut indices_rest) = (
            positions.as_mut_slice(),
            colors.as_mut_slice(),
            indices.as_mut_slice(),
        );
        let mut offset = 0u32;
        for part in parts {
            let count = part.positions.len();
            let (to_positions, rest) = positions_rest.split_at_mut(count);
            positions_rest = rest;
            let (to_colors, rest) = colors_rest.split_at_mut(count);
            colors_rest = rest;
            let (to_indices, rest) = indices_rest.split_at_mut(part.indices.len());
            indices_rest = rest;
            slots.push(Slot {
                part,
                positions: to_positions,
                colors: to_colors,
                indices: to_indices,
                offset,
            });
            offset += count as u32;
        }
        let copy = |slots: &mut [Slot]| {
            for slot in slots {
                slot.positions.copy_from_slice(&slot.part.positions);
                slot.colors.copy_from_slice(&slot.part.colors);
                for (to, from) in slot.indices.iter_mut().zip(&slot.part.indices) {
                    *to = from + slot.offset;
                }
            }
        };
        let per_group = slots.len().div_ceil(groups.max(1)).max(1);
        std::thread::scope(|scope| {
            let mut chunks = slots.chunks_mut(per_group);
            // первая группа — на вызывающем потоке: ему всё равно ждать
            let first = chunks.next();
            for chunk in chunks {
                scope.spawn(move || copy(chunk));
            }
            if let Some(chunk) = first {
                copy(chunk);
            }
        });
        MeshBuilder {
            positions,
            colors,
            indices,
            skipped_polygons: parts.iter().map(|part| part.skipped_polygons).sum(),
            ..MeshBuilder::default()
        }
    }

    /// Зарезервировать место под `vertices` вершин и `indices` индексов —
    /// когда объём геометрии известен заранее хотя бы оценкой.
    pub fn reserve(&mut self, vertices: usize, indices: usize) {
        self.positions.reserve(vertices);
        self.colors.reserve(vertices);
        self.indices.reserve(indices);
        if let Some(ribbon) = &mut self.ribbon {
            ribbon.reserve(vertices);
        }
        if let Some(roof) = &mut self.roof {
            roof.reserve(vertices);
        }
        if let Some(crown) = &mut self.crown {
            crown.reserve(vertices);
        }
    }

    pub fn build(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        if let Some(ribbon) = self.ribbon {
            mesh.insert_attribute(ATTRIBUTE_RIBBON, ribbon);
        }
        if let Some(roof) = self.roof {
            mesh.insert_attribute(ATTRIBUTE_ROOF, roof);
        }
        if let Some(crown) = self.crown {
            mesh.insert_attribute(ATTRIBUTE_CROWN, crown);
        }
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

#[cfg(test)]
mod tests;
