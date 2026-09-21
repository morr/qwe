//! Сборка слитых 2D-мешей слоёв карты: тысячи полигонов OSM в один
//! `Mesh2d` с вершинными цветами (стоковый `ColorMaterial` их умножает;
//! `map::surface::SurfaceMaterial` — умножает и кладёт поверх фактуру).

use std::f32::consts::PI;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexAttribute, VertexFormat};
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

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

/// Вершина без фактуры — кайма, оборудование кровли. Нулевой код материала
/// гасит фактуру, и нулевые первые два числа тогда ни на что не влияют. Ни
/// стена, ни фронтон сюда больше не входят: у обоих код из [`WallKind`].
///
/// [`WallKind`]: crate::map::buildings::material::WallKind
const NO_ROOF: [f32; 4] = [0.0; 4];

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
/// вместе с зеркалом в `roof.wgsl`.
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
/// `roof.wgsl`: там ровно те же две операции, и других мест, где этот слот
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
    /// `roof.wgsl::wall_shade` (там ветка `Solid` зовётся `gable`). Пороги
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
    /// разбора лежит в `roof.wgsl::wall_shade`, а на стороне Rust —
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

/// Максимальное удлинение стыка ленты относительно полуширины. Контур кроны
/// полон почти встречных рёбер (впадины между фестонами), и там miter уходит
/// в длинный шип — при 1.5 стык вырождается в срез, шипов не видно.
const MITER_LIMIT: f32 = 1.5;

/// Допуск на стрелку хорды дуги, м. Шаг тесселяции считается от радиуса, а не
/// берётся константой: у аллеи (полуширина 1.75 м) выходит ~28° на хорду, у
/// магистрали (8 м) ~13°, и обе дуги одинаково гладкие на глаз.
///
/// Тем же допуском отсекаются веера на почти прямых изломах — см.
/// [`MeshBuilder::push_join_fan`].
const ARC_TOLERANCE: f32 = 0.05;

/// Потолок числа хорд в дуге — страховка от вырожденного радиуса.
const MAX_ARC_STEPS: usize = 12;

/// Кайма контура ([`MeshBuilder::push_inset_band`]) не шире этой доли его
/// толщины (`площадь / периметр`): у полосы толщина — половина ширины, и кайма
/// в 0.6 её с каждой стороны оставляет посреди полосы просвет заливки.
const RIM_THICKNESS_SHARE: f32 = 0.6;
/// Кайма тоньше не кладётся: не видна, а квадов на контур — столько же.
const MIN_RIM_WIDTH: f32 = 0.2;

/// «До разрыва» у ленты без единого разрыва: длина дуги плюс это. Разметке
/// нужна координата, растущая вдоль ленты (по ней идут штрихи), а гаснуть ей
/// негде — way продолжается с обоих концов.
const FAR_FROM_BREAKS: f32 = 1000.0;

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
    /// Сведение по биссектрисе с ограничением [`MITER_LIMIT`].
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

/// Координаты [`ATTRIBUTE_RIBBON`] для вершин веера.
#[derive(Clone, Copy)]
enum FanCoords {
    /// Стык: весь веер лежит на внешней стороне излома, «до разрыва» у него
    /// одно на все вершины — то же, что у точки пути.
    Join { side: f32, to_break: f32 },
    /// Торец: поперёк — проекция на `normal` ленты, «до разрыва» продолжается
    /// за точку пути линейно с наклоном `slope` последнего квада: в минус у
    /// тупика и перекрёстка (разметка на полудиске гаснет), в плюс там, где
    /// way продолжается следующим и линия идёт сквозь стык.
    Cap {
        outward: Vec2,
        normal: Vec2,
        at_end: f32,
        slope: f32,
    },
}

#[derive(Default)]
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
    /// `Some` — меш собирается для `buildings::material::RoofMaterial` и несёт
    /// [`ATTRIBUTE_ROOF`] на каждой вершине.
    roof: Option<Vec<[f32; 4]>>,
    /// Чем заполняется [`ATTRIBUTE_ROOF`] у геометрии, которая ляжет дальше
    /// ([`Self::set_roof`] / [`Self::set_wall`]); нули — без фактуры.
    frame: Frame,
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

    /// Раскладка полос лент, положенных после этого вызова: колея шейдера
    /// улиц лежит только по ней, и проезд без полос её не получает. Без
    /// координат поверхности раскладку некуда записать.
    pub fn set_lanes(&mut self, lanes: Option<LaneFrame>) {
        self.lanes = [lanes; 2];
    }

    /// Раскладка клина ([`Self::push_taper`]), плывущая от `from` у его
    /// начала к `to` у конца.
    pub fn set_lane_taper(&mut self, from: Option<LaneFrame>, to: Option<LaneFrame>) {
        self.lanes = [from, to];
    }

    /// Кровля, которой принадлежит геометрия после этого вызова; `None` —
    /// кайма, оборудование: всё, чему фактуры не полагается вовсе. Стена и
    /// гаражная лента сюда не ходят, у них [`Self::set_wall`].
    pub fn set_roof(&mut self, roof: Option<Roof>) {
        self.frame = Frame::Same(roof.map_or(NO_ROOF, Roof::encode));
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

    /// Координаты ленты — тест проверяет по ним, что легло в атрибут.
    #[cfg(test)]
    pub fn ribbon_coords_for_test(&self) -> Option<&[[f32; 4]]> {
        self.ribbon.as_deref()
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

    /// Накопленные цвета — тестам мягких краёв: «кайма гаснет» проверяется
    /// альфой на вершине, а по одним позициям её не отличить от ленты, которая
    /// просто шире.
    #[cfg(test)]
    pub fn colors_for_test(&self) -> &[[f32; 4]] {
        &self.colors
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
        self.indices
            .extend(template.indices.iter().map(|index| base + index));
    }

    /// Полилиния как цепочка квадов; каждый конец сегмента продлён на
    /// полширины, чтобы стыки перекрывались (как у старых дорог-спрайтов).
    pub fn push_polyline(&mut self, points: &[Vec2], width: f32, color: LinearRgba) {
        let half_width = width / 2.0;
        let (along, total) = arclengths(points, false);
        for (index, segment) in points.windows(2).enumerate() {
            let Some(direction) = (segment[1] - segment[0]).try_normalize() else {
                continue;
            };
            let extension = direction * half_width;
            let normal = direction.perp() * half_width;
            let from = segment[0] - extension;
            let to = segment[1] + extension;
            // «до торца» — по продлённым концам; излом этой функции на
            // середине пути тут может попасть внутрь квада, но режим оставлен
            // ради сравнения картинок, а не ради разметки
            let at_from = to_nearest_end(along[index] - half_width, total);
            let at_to = to_nearest_end(along[index + 1] + half_width, total);
            self.push_quad_full(
                [from + normal, from - normal, to - normal, to + normal],
                [color; 4],
                [
                    self.coords(half_width, at_from, half_width),
                    self.coords(-half_width, at_from, half_width),
                    self.coords(-half_width, at_to, half_width),
                    self.coords(half_width, at_to, half_width),
                ],
            );
        }
    }

    /// **Клин** — разомкнутая лента, ширина которой вдоль ломаной меняется
    /// линейно по длине дуги от `widths[0]` до `widths[1]`: переход между
    /// сечениями улицы (`roads::network::sections`), где до этого ширина
    /// менялась ступенькой.
    ///
    /// Торцы срезаны ровно по крайним точкам, изломы — общими вершинами по
    /// биссектрисе ([`miter_offsets`]), как у ленты без веера: клин короток, а
    /// круглый торец выпирал бы из-под более узкой соседней ленты. Координаты
    /// фактуры: раскладка полос — по доле пути между двумя из
    /// [`Self::set_lane_taper`], так что колея плывёт вместе с краями;
    /// «до разрыва» — линейно от `to_break[0]` до `to_break[1]`, продолжением
    /// срезанной ленты ([`to_break_beyond`]), чтобы колея гасла у того же узла.
    pub fn push_taper(
        &mut self,
        points: &[Vec2],
        widths: [f32; 2],
        to_break: [f32; 2],
        color: LinearRgba,
    ) {
        let path = merge_close_points(points, false, widths[0].min(widths[1]) / 4.0);
        if path.len() < 2 {
            return;
        }
        let (along, total) = arclengths(&path, false);
        if total <= 0.0 {
            return;
        }
        let miters = miter_offsets(&path, false, 1.0);
        let rgba = color.to_f32_array();
        let base = self.positions.len() as u32;
        for ((&point, &at), miter) in path.iter().zip(&along).zip(&miters) {
            let share = at / total;
            let half_width = (widths[0] + (widths[1] - widths[0]) * share) / 2.0;
            let to_break = to_break[0] + (to_break[1] - to_break[0]) * share;
            // раскладка полос плывёт вместе с краями: крайняя полоса
            // рождается из клина, а колея остаётся между линиями краски
            let lanes = match self.lanes {
                [Some(from), Some(to)] => Some(from.lerp(to, share)),
                [from, _] => from,
            };
            self.push_vertex(
                point + *miter * half_width,
                rgba,
                Self::coords_in(lanes, half_width, to_break, half_width),
            );
            self.push_vertex(
                point - *miter * half_width,
                rgba,
                Self::coords_in(lanes, -half_width, to_break, half_width),
            );
        }
        for index in 0..path.len() as u32 - 1 {
            let (left, right) = (base + 2 * index, base + 2 * index + 1);
            let (next_left, next_right) = (left + 2, right + 2);
            self.indices
                .extend([left, right, next_right, left, next_right, next_left]);
        }
    }

    /// Полоса под одну линию слоя краски (`roads/paint.rs`): лента полуширины
    /// `half_width` вдоль `points`, стыки — общими вершинами по биссектрисе,
    /// торцы — по крайним точкам. Полоса **шире самой линии**: линию с полом
    /// в пиксели, штрихами и сглаженным краем рисует шейдер краски по
    /// координатам полосы, а полоса лишь оставляет ему место на самом дальнем
    /// зуме, где линия ещё видна.
    ///
    /// В [`ATTRIBUTE_RIBBON`] — `[поперёк от линии, длина улицы, до разрыва,
    /// kind]` из `stations` (по одной на точку), в альфу цвета — видимость
    /// станции.
    pub fn push_paint_strip(
        &mut self,
        points: &[Vec2],
        closed: bool,
        half_width: f32,
        stations: &[PaintStation],
        kind: f32,
        color: LinearRgba,
    ) {
        let count = points.len();
        if count < 2 || stations.len() != count {
            return;
        }
        let miters = miter_offsets(points, closed, half_width);
        let base = self.positions.len() as u32;
        for ((&point, miter), station) in points.iter().zip(&miters).zip(stations) {
            let rgba = color.with_alpha(color.alpha * station.alpha).to_f32_array();
            for side in [1.0, -1.0] {
                self.push_vertex(
                    point + *miter * side,
                    rgba,
                    [side * half_width, station.along, station.to_break, kind],
                );
            }
        }
        let segments = if closed { count } else { count - 1 };
        for index in 0..segments as u32 {
            let next = (index + 1) % count as u32;
            let (left, right) = (base + 2 * index, base + 2 * index + 1);
            let (next_left, next_right) = (base + 2 * next, base + 2 * next + 1);
            self.indices
                .extend([left, right, next_right, left, next_right, next_left]);
        }
    }

    /// Лента постоянной ширины вдоль ломаной со стыками по биссектрисе
    /// (miter с ограничением `MITER_LIMIT`) и торцами по последней точке.
    /// Для тонких контуров `push_polyline` не годится — там каждый сегмент
    /// продлён на полширины, и на ломаной с сегментами короче ширины штриха
    /// (контур кроны) продления соседних квадов торчат наружу шипами.
    pub fn push_stroke(&mut self, points: &[Vec2], closed: bool, width: f32, color: LinearRgba) {
        self.push_ribbon(
            points,
            closed,
            width,
            color,
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }

    /// Пунктир вдоль ломаной: лента ширины `width` кусками по `dash` метров
    /// через `gap`. Так Mapnik рисует ж/д путь в osm-carto — рисунок оттуда,
    /// цвет штриха выбирает вызывающий.
    ///
    /// Один проход по сегментам с курсором по длине дуги: точки текущего штриха
    /// копятся на ходу (концы интерполируются, вершины OSM между ними
    /// сохраняются) и сбрасываются в ленту по завершении штриха. Торцы — `Butt`:
    /// штрих это метка, а не конец дороги.
    ///
    /// Путь короче одного штриха всё равно даёт один штрих: иначе короткие ways
    /// (а их в ж/д развязке большинство) остались бы голой лентой без метки.
    pub fn push_dashes(
        &mut self,
        points: &[Vec2],
        width: f32,
        dash: f32,
        gap: f32,
        color: LinearRgba,
        join: RibbonJoin,
    ) {
        if dash <= 0.0 || gap <= 0.0 || points.len() < 2 {
            return;
        }

        // остаток текущего интервала и что это за интервал
        let mut left = dash;
        let mut drawing = true;
        let mut current = vec![points[0]];

        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let Some(direction) = (to - from).try_normalize() else {
                continue;
            };
            let mut remaining = from.distance(to);
            let mut cursor = from;

            while remaining > left {
                cursor += direction * left;
                remaining -= left;
                if drawing {
                    current.push(cursor);
                    self.push_ribbon(&current, false, width, color, join, RibbonCap::Butt);
                }
                // буфер один на весь путь: своя `Vec` на каждый штрих ж/д
                // развязки — это аллокация на каждые несколько метров пути
                current.clear();
                current.push(cursor);
                drawing = !drawing;
                left = if drawing { dash } else { gap };
            }

            left -= remaining;
            // в пропуске копить нечего: следующий штрих начнётся с точки,
            // которую поставит переключение внутри цикла выше
            if drawing {
                current.push(to);
            }
        }

        if drawing && current.len() > 1 {
            self.push_ribbon(&current, false, width, color, join, RibbonCap::Butt);
        }
    }

    /// Поперечные шпалы вдоль ломаной: через каждые `spacing` метров — планка
    /// длиной `length` поперёк пути и толщиной `thickness`. Так рисуют трамвай
    /// Яндекс.Карты и 2ГИС — тонкая линия с частой поперечной насечкой.
    ///
    /// Тот же проход по длине дуги, что и у [`Self::push_dashes`], только на
    /// отметке ставится не кусок пути, а перпендикуляр к нему. Первая шпала
    /// отступает на полшага: планка ровно в торце пути выглядит обрубком, а на
    /// стыке двух ways две такие складываются в крест.
    pub fn push_ticks(
        &mut self,
        points: &[Vec2],
        length: f32,
        thickness: f32,
        spacing: f32,
        color: LinearRgba,
    ) {
        if spacing <= 0.0 || length <= 0.0 || thickness <= 0.0 || points.len() < 2 {
            return;
        }

        let half = length / 2.0;
        let mut left = spacing / 2.0;

        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let Some(direction) = (to - from).try_normalize() else {
                continue;
            };
            let mut remaining = from.distance(to);
            let mut cursor = from;

            while remaining > left {
                cursor += direction * left;
                remaining -= left;
                let arm = direction.perp() * half;
                self.push_ribbon(
                    &[cursor - arm, cursor + arm],
                    false,
                    thickness,
                    color,
                    RibbonJoin::Miter,
                    RibbonCap::Butt,
                );
                left = spacing;
            }

            left -= remaining;
        }
    }

    /// Две нитки рельсов вдоль ломаной: ленты ширины `width` по обе стороны
    /// осевой, на полколеи от неё. Смещение — тот же miter-офсет, которым
    /// считается край ленты ([`miter_offsets`]), поэтому нитка повторяет
    /// изгиб пути без щелей на изломах и на самом изломе держит колею, а не
    /// уезжает наружу вместе с углом.
    ///
    /// С балластом у нитки общая осевая, но своя склейка близких точек:
    /// `gauge / 4` здесь против `width / 4` у ленты ([`Self::push_ribbon`]),
    /// так что на очень частой ломаной балласт срезает угол хордой там, где
    /// нитка ещё идёт по точкам. Расхождение ограничено порогом склейки
    /// ленты — на радиусах реальных кривых это сантиметры.
    ///
    /// Торцы — `Butt`: нитка кончается там же, где way, а полудиск на
    /// сантиметровой ленте не виден и стоит лишнего веера.
    pub fn push_rails(
        &mut self,
        points: &[Vec2],
        gauge: f32,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
    ) {
        if gauge <= 0.0 || width <= 0.0 {
            return;
        }
        // склейка по колее, а не по ширине нитки: нитка тоньше сантиметров, и
        // по её мерке в путь прошли бы точки, вырождающие нормаль офсета
        let path = merge_close_points(points, false, gauge / 4.0);
        if path.len() < 2 {
            return;
        }

        let offsets = miter_offsets(&path, false, gauge / 2.0);
        let mut line = Vec::with_capacity(path.len());
        for side in [1.0_f32, -1.0] {
            line.clear();
            line.extend(
                path.iter()
                    .zip(&offsets)
                    .map(|(point, offset)| *point + *offset * side),
            );
            self.push_ribbon(&line, false, width, color, join, RibbonCap::Butt);
        }
    }

    /// Лента постоянной ширины вдоль ломаной: `join` — чем закрыт излом,
    /// `cap` — чем закрыты торцы разомкнутой ленты.
    ///
    /// Точки ближе `width / 4` к предыдущей отбрасываются: на такой дистанции
    /// они не видны, но вырождают нормаль стыка.
    pub fn push_ribbon(
        &mut self,
        points: &[Vec2],
        closed: bool,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
        cap: RibbonCap,
    ) {
        self.push_ribbon_capped(points, closed, width, color, join, [cap; 2]);
    }

    /// То же, но торцы задаются по отдельности — `[начало, конец]`. Русло,
    /// которому это нужно (открытый конец против входа в трубу), кладётся уже
    /// через [`Self::push_ribbon_broken`]: отмели на его кромках нужна своя
    /// координата «до разрыва» (`water::mesh_water_lines`).
    pub fn push_ribbon_capped(
        &mut self,
        points: &[Vec2],
        closed: bool,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
        caps: [RibbonCap; 2],
    ) {
        self.push_ribbon_shaped(
            points,
            width,
            color,
            RibbonShape {
                closed,
                join,
                caps,
                breaks: RibbonBreaks::Ends,
            },
        );
    }

    /// Лента заданной [`RibbonShape`] — вход для тех, кому мало `join`+`cap`:
    /// проезжей части улицы с её разрывами разметки ([`RibbonBreaks`]) и
    /// кольца, которое рисуется замкнутым. «До разрыва» в [`ATTRIBUTE_RIBBON`]
    /// считается до края ближайшего разрыва, а не до торца, и у замкнутой
    /// ленты — **по кругу**.
    pub fn push_ribbon_shaped(
        &mut self,
        points: &[Vec2],
        width: f32,
        color: LinearRgba,
        shape: RibbonShape,
    ) {
        let RibbonShape {
            closed,
            join,
            caps,
            breaks,
        } = shape;
        let mut path = merge_close_points(points, closed, width / 4.0);
        if path.len() < 2 {
            return;
        }

        let half_width = width / 2.0;
        let (mut along, total) = arclengths(&path, closed);
        // «до разрыва» — по ней шейдер гасит разметку у перекрёстка и
        // фазирует штрихи; у замкнутой ленты разрывов нет, остаётся длина дуги
        let gaps = GapProfile::new(&path, &along, total, breaks, closed.then_some(total));
        if self.ribbon.is_some() {
            gaps.split_path(&mut path, &mut along, width / 4.0);
        }
        let ends: Vec<f32> = along.iter().map(|&at| gaps.distance(at)).collect();

        let count = path.len();
        let segments = if closed { count } else { count - 1 };

        match join {
            RibbonJoin::Miter => {
                let offsets = miter_offsets(&path, closed, half_width);

                for index in 0..segments {
                    let next = (index + 1) % count;
                    self.push_quad_full(
                        [
                            path[index] + offsets[index],
                            path[index] - offsets[index],
                            path[next] - offsets[next],
                            path[next] + offsets[next],
                        ],
                        [color; 4],
                        self.segment_coords(half_width, ends[index], ends[next]),
                    );
                }
            }
            RibbonJoin::Round => {
                // Сегменты — квады без продлений; на внутренней стороне
                // излома они перекрываются сами, снаружи щель закрывает веер.
                // Излом, которому веер не положен (щель мельче допуска), квады
                // проходят **общими вершинами** по биссектрисе: торцы со своими
                // нормалями там не совпадают ни в одной точке, кроме осевой, и
                // растеризатор оставлял между ними волосяную щель поперёк всей
                // ленты — светлую линию тротуара под проездом.
                // направления до и после каждой вершины: `None` у концов
                // открытой ленты и у вершины с вырожденным звеном
                let bends: Vec<Option<(Vec2, Vec2)>> = (0..count)
                    .map(|index| {
                        if !closed && (index == 0 || index + 1 == count) {
                            return None;
                        }
                        let previous = path[(index + count - 1) % count];
                        let next = path[(index + 1) % count];
                        Some((
                            (path[index] - previous).try_normalize()?,
                            (next - path[index]).try_normalize()?,
                        ))
                    })
                    .collect();
                let miters = miter_offsets(&path, closed, half_width);
                let shared: Vec<bool> = bends
                    .iter()
                    .map(|bend| {
                        bend.is_some_and(|(incoming, outgoing)| {
                            join_gap_hidden(half_width, incoming.angle_to(outgoing))
                        })
                    })
                    .collect();
                for index in 0..segments {
                    let next = (index + 1) % count;
                    let Some(direction) = (path[next] - path[index]).try_normalize() else {
                        continue;
                    };
                    let normal = direction.perp() * half_width;
                    let at_start = if shared[index] { miters[index] } else { normal };
                    let at_end = if shared[next] { miters[next] } else { normal };
                    self.push_quad_full(
                        [
                            path[index] + at_start,
                            path[index] - at_start,
                            path[next] - at_end,
                            path[next] + at_end,
                        ],
                        [color; 4],
                        self.segment_coords(half_width, ends[index], ends[next]),
                    );
                }
                for (index, bend) in bends.iter().enumerate() {
                    let Some((incoming, outgoing)) = *bend else {
                        continue;
                    };
                    self.push_join_fan(
                        path[index],
                        half_width,
                        incoming,
                        outgoing,
                        color,
                        ends[index],
                    );
                }
            }
        }

        if !closed {
            // полудиск за начальной точкой: от нормали через −direction,
            // то есть назад по ходу пути
            if caps[0] == RibbonCap::Round
                && let Some(direction) = (path[1] - path[0]).try_normalize()
            {
                self.push_arc_fan(
                    path[0],
                    half_width,
                    direction.perp().to_angle(),
                    PI,
                    color,
                    FanCoords::Cap {
                        outward: -direction,
                        normal: direction.perp(),
                        at_end: ends[0],
                        slope: slope_between(ends[0], ends[1], along[1] - along[0]),
                    },
                );
            }
            if caps[1] == RibbonCap::Round
                && let Some(direction) = (path[count - 1] - path[count - 2]).try_normalize()
            {
                self.push_arc_fan(
                    path[count - 1],
                    half_width,
                    (-direction.perp()).to_angle(),
                    PI,
                    color,
                    FanCoords::Cap {
                        outward: direction,
                        normal: direction.perp(),
                        at_end: ends[count - 1],
                        slope: slope_between(
                            ends[count - 1],
                            ends[count - 2],
                            along[count - 1] - along[count - 2],
                        ),
                    },
                );
            }
        }
    }

    /// Координаты четырёх углов квада сегмента: `+нормаль` в начале, `−нормаль`
    /// в начале, `−нормаль` в конце, `+нормаль` в конце — порядок
    /// [`Self::push_quad_full`].
    fn segment_coords(&self, half_width: f32, at_start: f32, at_end: f32) -> [[f32; 4]; 4] {
        [
            self.coords(half_width, at_start, half_width),
            self.coords(-half_width, at_start, half_width),
            self.coords(-half_width, at_end, half_width),
            self.coords(half_width, at_end, half_width),
        ]
    }

    /// Веер, закрывающий щель butt-квадов на **внешней** стороне излома.
    /// При левом повороте (`angle_to > 0`) щель справа по ходу, и наоборот.
    ///
    /// Порог пропуска — по **ширине щели** (`радиус · излом`), а не по углу:
    /// один и тот же излом в 5° у аллеи оставляет 15 см, и на приближении это
    /// хорошо видимая светлая прорезь поперёк дороги. Тот же допуск, что и на
    /// стрелку хорды дуги, — то, чего не видно, одинаково не видно и там, и
    /// тут; изломы медианной для Тулы крутизны (3.4°) веер всё равно получают.
    fn push_join_fan(
        &mut self,
        center: Vec2,
        radius: f32,
        incoming: Vec2,
        outgoing: Vec2,
        color: LinearRgba,
        to_break: f32,
    ) {
        let turn = incoming.angle_to(outgoing);
        if join_gap_hidden(radius, turn) {
            return;
        }
        let side = -turn.signum();
        // угол нормали растёт вместе с углом направления, поэтому от нормали
        // входящего сегмента до нормали исходящего ровно `turn` радиан
        let start = (incoming.perp() * side).to_angle();
        self.push_arc_fan(
            center,
            radius,
            start,
            turn,
            color,
            FanCoords::Join { side, to_break },
        );
    }

    /// Веер треугольников по дуге: `sweep` радиан от `start` вокруг `center`.
    fn push_arc_fan(
        &mut self,
        center: Vec2,
        radius: f32,
        start: f32,
        sweep: f32,
        color: LinearRgba,
        coords: FanCoords,
    ) {
        let steps = arc_steps(radius, sweep.abs());
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        let at_center = match coords {
            FanCoords::Join { to_break, .. } => self.coords(0.0, to_break, radius),
            FanCoords::Cap { at_end, .. } => self.coords(0.0, at_end, radius),
        };
        self.push_vertex(center, rgba, at_center);
        for step in 0..=steps {
            let angle = start + sweep * step as f32 / steps as f32;
            let point = center + Vec2::from_angle(angle) * radius;
            let at_rim = match coords {
                FanCoords::Join { side, to_break } => self.coords(side * radius, to_break, radius),
                FanCoords::Cap {
                    outward,
                    normal,
                    at_end,
                    slope,
                } => {
                    let offset = point - center;
                    self.coords(
                        offset.dot(normal),
                        at_end + slope * offset.dot(outward),
                        radius,
                    )
                }
            };
            self.push_vertex(point, rgba, at_rim);
        }
        for step in 0..steps as u32 {
            self.indices
                .extend([base, base + 1 + step, base + 2 + step]);
        }
    }

    /// Кайма вдоль замкнутого контура: полоса ширины `width` от контура вглубь
    /// него (при `outside` — наружу, так каймится дырка), цвет от `edge` на
    /// контуре до `inner` на дальнем краю. Мелководье у берега, тёмная кромка
    /// луга. Дальний край строится miter-офсетами, поэтому на острых вогнутых
    /// углах соседние квады накладываются — цвет там один и тот же, и наложение
    /// не видно.
    ///
    /// Ширина зажата толщиной самого контура (`площадь / периметр` — у полосы
    /// это половина её ширины): у газона-разделителя в полтора метра кайма в два
    /// метра вылезла бы за дальний край на дорогу. Тоньше 20 см кайма не кладётся.
    /// Для `outside` толщина дырки ни при чём — ширину зажимает вызывающий по
    /// внешнему контуру. Возвращает положенную ширину, `None` — кайма не легла.
    pub fn push_inset_band(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        edge: LinearRgba,
        inner: LinearRgba,
    ) -> Option<f32> {
        self.push_inset_band_with(ring, width, outside, |_, _| (edge, inner))
    }

    /// Та же кайма, но цвет решается на каждое ребро контура `(a, b)`: одной
    /// парой цветов на весь контур не сказать, что грань, повёрнутая к солнцу,
    /// светлее отвёрнутой. Слои зданий этим сейчас не пользуются — так был
    /// сделан парапет кровли, которого больше нет (`layers.rs`).
    pub fn push_inset_band_with(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        color: impl Fn(Vec2, Vec2) -> (LinearRgba, LinearRgba),
    ) -> Option<f32> {
        self.push_band(ring, width, outside, |_| 1.0, color)
    }

    /// Та же кайма, но ширина решается на каждой вершине — по единичному
    /// направлению, в котором кайма от неё пойдёт. Полутень тени растёт с
    /// расстоянием от того, кто её отбрасывает: у самой стены край жёсткий,
    /// вдали — размытый, и одной ширины на весь контур этого не сказать
    /// ([`super::buildings::shadows::shadow_builder`]).
    ///
    /// `taper` возвращает долю `width` (вне `0..=1` зажимается). Схлопнутая в
    /// ноль вершина вырождает свой квад в треугольник — это и есть жёсткий
    /// край; ребро, у которого схлопнуты обе, не кладётся вовсе.
    pub fn push_inset_band_tapered(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        taper: impl Fn(Vec2) -> f32,
        edge: LinearRgba,
        inner: LinearRgba,
    ) -> Option<f32> {
        self.push_band(ring, width, outside, taper, |_, _| (edge, inner))
    }

    fn push_band(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        taper: impl Fn(Vec2) -> f32,
        color: impl Fn(Vec2, Vec2) -> (LinearRgba, LinearRgba),
    ) -> Option<f32> {
        let path = merge_close_points(ring, true, width / 4.0);
        if path.len() < 3 {
            return None;
        }
        let area = signed_area(&path);
        let width = if outside {
            width
        } else {
            width.min(RIM_THICKNESS_SHARE * area.abs() / perimeter(&path))
        };
        if width < MIN_RIM_WIDTH {
            return None;
        }
        // у обхода против часовой стрелки внутренняя сторона слева — куда и
        // смотрят miter-офсеты; по часовой — справа
        let side = if (area > 0.0) != outside { 1.0 } else { -1.0 };
        // офсеты единичной ширины, чтобы `taper` домножал уже готовое
        // направление: множитель длину меняет, направление — нет. Сторона
        // тоже вносится здесь, на месте — лишний `Vec` на кольцо стоил бы
        // дороже самого сужения
        let mut offsets = miter_offsets(&path, true, 1.0);
        for offset in &mut offsets {
            let direction = *offset * side;
            *offset = direction * width * taper(direction.normalize_or_zero()).clamp(0.0, 1.0);
        }
        let count = path.len();
        for index in 0..count {
            let next = (index + 1) % count;
            if offsets[index] == Vec2::ZERO && offsets[next] == Vec2::ZERO {
                continue;
            }
            let (edge, inner) = color(path[index], path[next]);
            self.push_quad_gradient(
                [
                    path[index],
                    path[index] + offsets[index],
                    path[next] + offsets[next],
                    path[next],
                ],
                [edge, inner, inner, edge],
            );
        }
        Some(width)
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
        /// Вершина со всем, что на ней едет: позиция (с z), цвет и две
        /// необязательные четвёрки фактуры.
        type Vertex = ([f32; 3], [f32; 4], [f32; 4], [f32; 4]);
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
        self.indices = indices;
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
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// Минимальный по площади описанный прямоугольник кольца, CCW, первое ребро
/// вдоль длинной оси. У такого прямоугольника одна сторона лежит на ребре
/// выпуклой оболочки, а рёбра оболочки — подмножество рёбер контура, так что
/// перебор направлений всех рёбер находит оптимум без построения оболочки:
/// контуров тысячи, вершин в каждом — единицы.
pub fn min_area_rect(ring: &[Vec2]) -> Option<[Vec2; 4]> {
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

/// Площадь замкнутого контура по формуле шнурков, со знаком: положительна при
/// обходе против часовой стрелки. Делить на два обязательно — на «площадь /
/// периметр» как на толщину контура опирается зажим каймы
/// ([`MeshBuilder::push_inset_band`]).
///
/// Та же величина, что даёт `osm::model::signed_ring_area`; копия здесь ровно
/// потому, что `meshing` не знает про OSM-модель и не должен узнать — это
/// чистая геометрия.
fn signed_area(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].perp_dot(ring[(index + 1) % count]))
        .sum::<f32>()
        / 2.0
}

/// Периметр замкнутого контура.
fn perimeter(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].distance(ring[(index + 1) % count]))
        .sum()
}

/// Длина дуги в каждой точке ломаной и полная длина; у замкнутой — с
/// замыкающим сегментом.
fn arclengths(path: &[Vec2], closed: bool) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(path.len());
    let mut total = 0.0;
    for (index, point) in path.iter().enumerate() {
        if index > 0 {
            total += point.distance(path[index - 1]);
        }
        along.push(total);
    }
    if closed && path.len() > 1 {
        total += path[path.len() - 1].distance(path[0]);
    }
    (along, total)
}

/// «До разрыва» разомкнутой ленты [`MeshBuilder::push_ribbon_shaped`] с
/// разрывами `breaks` на её **продолжении** — `beyond` метров за началом
/// (`end = false`) или концом пути. Нужна клину (`MeshBuilder::push_taper`),
/// который продолжает срезанную ленту: с тем же значением на стыке штрихи
/// разметки идут через стык без сдвига фазы.
pub fn to_break_beyond(
    points: &[Vec2],
    width: f32,
    breaks: &[Break],
    end: bool,
    beyond: f32,
) -> f32 {
    let path = merge_close_points(points, false, width / 4.0);
    if path.len() < 2 {
        return FAR_FROM_BREAKS;
    }
    let (along, total) = arclengths(&path, false);
    let gaps = GapProfile::new(&path, &along, total, RibbonBreaks::At(breaks), None);
    gaps.distance(if end { total + beyond } else { -beyond })
}

/// Путь ленты с вершинами на изломах «до разрыва», длина дуги и «до разрыва»
/// в каждой его вершине — то, что [`MeshBuilder::push_ribbon_shaped`] кладёт в
/// атрибут проезжей части, для слоя краски (`roads/paint.rs`): тот строит линии
/// поверх ленты сам и обязан гаснуть у тех же узлов. Точки ближе
/// `merge_distance` сливаются, как у ленты. Путь короче двух точек — пустые
/// списки.
pub fn break_profile(
    points: &[Vec2],
    closed: bool,
    breaks: &[Break],
    merge_distance: f32,
) -> (Vec<Vec2>, Vec<f32>, Vec<f32>) {
    let mut path = merge_close_points(points, closed, merge_distance);
    if path.len() < 2 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let (mut along, total) = arclengths(&path, closed);
    let gaps = GapProfile::new(
        &path,
        &along,
        total,
        RibbonBreaks::At(breaks),
        closed.then_some(total),
    );
    gaps.split_path(&mut path, &mut along, merge_distance);
    let to_break = along.iter().map(|&at| gaps.distance(at)).collect();
    (path, along, to_break)
}

/// Расстояние до ближайшего торца разомкнутого пути.
fn to_nearest_end(along: f32, total: f32) -> f32 {
    along.min(total - along)
}

/// Наклон «до разрыва» на последнем кваде — чтобы продолжить её за торец.
/// Вырожденный квад (нулевой длины) наклона не имеет: за торцом тогда
/// как у тупика, в минус.
fn slope_between(at_end: f32, at_neighbour: f32, distance: f32) -> f32 {
    if distance > 0.0 {
        (at_end - at_neighbour) / distance
    } else {
        -1.0
    }
}

/// Форма ленты: замкнута ли, чем закрыты изломы и торцы, где разрывы разметки.
///
/// У замкнутой (`closed`) торцов нет вовсе — ни полудисков, ни продлений:
/// последняя точка пути повторяет первую, [`merge_close_points`] её снимает, а
/// шов получает такой же веер излома, как всякая другая вершина.
pub struct RibbonShape<'a> {
    pub closed: bool,
    pub join: RibbonJoin,
    pub caps: [RibbonCap; 2],
    pub breaks: RibbonBreaks<'a>,
}

/// Разрывы разметки одной ленты в координатах длины дуги — отсортированы и не
/// пересекаются, так что между двумя соседними «до разрыва» — функция с одним
/// изломом.
struct GapProfile {
    /// `(центр, полудлина)` каждого разрыва.
    gaps: Vec<(f32, f32)>,
    /// Длина замкнутой ленты: по ней «до разрыва» меряется **по кругу**, в обе
    /// стороны от точки. `None` у разомкнутой.
    period: Option<f32>,
}

impl GapProfile {
    fn new(
        path: &[Vec2],
        along: &[f32],
        total: f32,
        breaks: RibbonBreaks,
        period: Option<f32>,
    ) -> Self {
        let mut gaps: Vec<(f32, f32)> = match breaks {
            // у кольца торцов нет: шов — не разрыв, и гасить у него разметку
            // значило бы провести поперёк кольца черту на пустом месте
            RibbonBreaks::Ends if period.is_some() => Vec::new(),
            RibbonBreaks::Ends => vec![(0.0, 0.0), (total, 0.0)],
            RibbonBreaks::At(list) => list
                .iter()
                .map(|gap| {
                    (
                        project_onto_path(path, along, gap.at, period.is_some()),
                        gap.reach,
                    )
                })
                .collect(),
        };
        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
        // пересекающиеся разрывы — в один: иначе между ними ближайший менялся
        // бы не там, где считает `kinks`
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(gaps.len());
        for (center, reach) in gaps {
            match merged.last_mut() {
                Some(last) if center - reach <= last.0 + last.1 => {
                    let start = (last.0 - last.1).min(center - reach);
                    let end = (last.0 + last.1).max(center + reach);
                    *last = ((start + end) / 2.0, (end - start) / 2.0);
                }
                _ => merged.push((center, reach)),
            }
        }
        Self {
            gaps: merged,
            period,
        }
    }

    /// Дуга от точки до центра разрыва — у замкнутой ленты **короткая из
    /// двух**: по кольцу к разрыву идут в обе стороны.
    fn span(&self, at: f32, center: f32) -> f32 {
        let gap = (at - center).abs();
        match self.period {
            Some(period) => gap.min(period - gap),
            None => gap,
        }
    }

    /// Знаковое расстояние до края ближайшего разрыва: внутри разрыва
    /// отрицательно. Без разрывов — длина дуги за [`FAR_FROM_BREAKS`]: штрихам
    /// нужна растущая координата. У замкнутой ленты без разрывов она на шве
    /// прыгает на всю длину кольца, то есть один штрих там не совпадёт фазой,
    /// — но размеченного кольца не бывает: `roads::lane_count` даёт кольцу
    /// одну полосу, а однополосной разметки нет.
    fn distance(&self, at: f32) -> f32 {
        if self.gaps.is_empty() {
            return at + FAR_FROM_BREAKS;
        }
        self.gaps
            .iter()
            .map(|&(center, reach)| self.span(at, center) - reach)
            .fold(f32::INFINITY, f32::min)
    }

    /// Изломы функции «до разрыва»: центр каждого разрыва и точка между
    /// соседними, где ближайший из них меняется. У замкнутой ленты соседи
    /// считаются по кругу, так что последний разрыв граничит с первым **через
    /// шов**; единственный разрыв кольца граничит сам с собой в противоположной
    /// точке, где расстояние перестаёт расти, — квад, накрывший её без вершины,
    /// увёл бы разметку за разрыв, которого там нет.
    fn kinks(&self) -> Vec<f32> {
        let mut kinks = Vec::with_capacity(self.gaps.len() * 2);
        for (index, &(center, reach)) in self.gaps.iter().enumerate() {
            kinks.push(center);
            if let Some(&(next_center, next_reach)) = self.gaps.get(index + 1) {
                kinks.push((center + next_center + reach - next_reach) / 2.0);
            }
        }
        if let (Some(period), Some(&(first, first_reach)), Some(&(last, last_reach))) =
            (self.period, self.gaps.first(), self.gaps.last())
        {
            let crossover = (last + first + period + last_reach - first_reach) / 2.0;
            kinks.push(crossover.rem_euclid(period));
        }
        kinks
    }

    /// Вершина на каждом изломе, чтобы «до разрыва» была линейной внутри
    /// каждого квада: GPU интерполирует атрибут по прямой, и квад, накрывший
    /// излом, получил бы расстояние, растущее там, где оно убывает. Излом ближе
    /// `merge_distance` к соседней вершине не вставляется: внутри такого
    /// коротышки его не видно, а вырожденный квад — да.
    fn split_path(&self, path: &mut Vec<Vec2>, along: &mut Vec<f32>, merge_distance: f32) {
        for kink in self.kinks() {
            if insert_vertex_at(path, along, kink, merge_distance) {
                continue;
            }
            // излом на замыкающем звене: вершин по обе стороны от него нет,
            // оно идёт от последней точки к первой — такая вершина
            // дописывается в хвост
            if let Some(period) = self.period {
                append_vertex_at(path, along, kink, merge_distance, period);
            }
        }
    }
}

/// Вершина на длине дуги `at`, если та внутри какого-то сегмента и не ближе
/// `merge_distance` к его концам. Отвечает, нашёлся ли такой сегмент, — чтобы
/// звонящий знал, что остаётся замыкающее звено кольца ([`append_vertex_at`]).
fn insert_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
) -> bool {
    let Some(index) = along
        .windows(2)
        .position(|pair| pair[0] < at && at < pair[1])
    else {
        return false;
    };
    if at - along[index] <= merge_distance || along[index + 1] - at <= merge_distance {
        return true;
    }
    let t = (at - along[index]) / (along[index + 1] - along[index]);
    let point = path[index].lerp(path[index + 1], t);
    path.insert(index + 1, point);
    along.insert(index + 1, at);
    true
}

/// То же на **замыкающем** звене кольца — от последней точки к первой: вершина
/// там не вставляется между соседями, а дописывается в хвост.
fn append_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
    period: f32,
) {
    let (Some(&last), Some(&first)) = (along.last(), path.first()) else {
        return;
    };
    if at <= last || at >= period {
        return;
    }
    if at - last <= merge_distance || period - at <= merge_distance {
        return;
    }
    let t = (at - last) / (period - last);
    let tail = path[path.len() - 1];
    path.push(tail.lerp(first, t));
    along.push(at);
}

/// Длина дуги в ближайшей к `point` точке ломаной.
fn project_onto_path(path: &[Vec2], along: &[f32], point: Vec2, closed: bool) -> f32 {
    let mut best = (f32::INFINITY, 0.0);
    // у замкнутого пути сегментов столько же, сколько точек: последний идёт
    // от хвоста к голове
    let count = path.len();
    let segments = if closed && count > 1 {
        count
    } else {
        count.saturating_sub(1)
    };
    for index in 0..segments {
        let (from, to) = (path[index], path[(index + 1) % count]);
        let span = to - from;
        let length_sq = span.length_squared();
        let t = if length_sq > 0.0 {
            ((point - from).dot(span) / length_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = point.distance_squared(from + span * t);
        if distance < best.0 {
            best = (distance, along[index] + t * length_sq.sqrt());
        }
    }
    best.1
}

/// Свип выпуклого контура по свету: оболочка контура и его копии, сдвинутой
/// на `offset`, — ровно то, что накрывает тень. Общий для машин
/// (`cars/body.rs`) и оград (`fences.rs`): тень начинается **под** объектом и
/// вытекает из-под него, а не лежит сдвинутой копией в стороне.
///
/// Строится обходом, без сортировки: ребро, чья внешняя нормаль смотрит по
/// свету, уезжает на `offset`, остальные остаются на месте, а в двух
/// вершинах, где одно сменяется другим, оболочка переходит из одной копии в
/// другую. Это сумма Минковского контура с отрезком `[0, offset]`, поэтому
/// на выпуклом контуре результат выпуклый — на это опирается `push_convex`.
/// Контур — против часовой. Нулевой `offset` (солнце в зените) даёт обратно
/// сам контур.
pub fn sweep_convex(outline: &[Vec2], offset: Vec2) -> Vec<Vec2> {
    let count = outline.len();
    // контур обходится против часовой, значит внешняя нормаль ребра смотрит
    // вправо от него; ребро отбрасывает тень наружу, когда она смотрит по свету
    let casts = |at: usize| {
        let edge = outline[(at + 1) % count] - outline[at];
        Vec2::new(edge.y, -edge.x).dot(offset) > 0.0
    };
    let mut hull = Vec::with_capacity(count + 2);
    for (at, &point) in outline.iter().enumerate() {
        match (casts((at + count - 1) % count), casts(at)) {
            (false, false) => hull.push(point),
            (true, true) => hull.push(point + offset),
            (false, true) => hull.extend([point, point + offset]),
            (true, false) => hull.extend([point + offset, point]),
        }
    }
    hull
}

/// Miter-офсет вершины ломаной: вектор от точки пути до края ленты полуширины
/// `half_width`, по биссектрисе излома, с ограничением [`MITER_LIMIT`] на
/// острых стыках. Край ленты — `path[i] ± offsets[i]`. Общий и для отрисовки
/// ([`MeshBuilder::push_ribbon`]), и для навмеша (бордюры моста): полосы,
/// заблокированные в сетке, совпадают с нарисованными по построению.
pub fn miter_offsets(path: &[Vec2], closed: bool, half_width: f32) -> Vec<Vec2> {
    let count = path.len();
    (0..count)
        .map(|index| {
            let incoming = (index > 0 || closed).then(|| {
                let previous = path[(index + count - 1) % count];
                (path[index] - previous).normalize_or(Vec2::X).perp()
            });
            let outgoing = (index + 1 < count || closed).then(|| {
                let next = path[(index + 1) % count];
                (next - path[index]).normalize_or(Vec2::X).perp()
            });
            match (incoming, outgoing) {
                (Some(before), Some(after)) => {
                    let bisector = (before + after).normalize_or(before);
                    // на острых стыках длина miter уходит в бесконечность — режем
                    let cosine = bisector.dot(before).max(1.0 / MITER_LIMIT);
                    bisector * (half_width / cosine)
                }
                (Some(normal), None) | (None, Some(normal)) => normal * half_width,
                (None, None) => Vec2::ZERO,
            }
        })
        .collect()
}

/// Ломаная без точек ближе `merge_distance` к предыдущей: на такой дистанции
/// они не видны, но вырождают нормаль стыка. У замкнутой ленты так же
/// подрезается хвост, сошедшийся с началом.
///
/// Эпсилон выбирает потребитель, и они разные: рендеру важно «не видно»
/// (`width / 4`, [`MeshBuilder::push_ribbon`]), полигональному навмешу — только
/// вырожденность (`polymesh::build::DEGENERATE_SPAN`), потому что схлопывание
/// по ширине сдвинуло бы его футпринт относительно заливки сетки.
pub fn merge_close_points(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<Vec2> {
    let merge_distance_sq = merge_distance.powi(2);
    let mut path: Vec<Vec2> = Vec::with_capacity(points.len());
    for &point in points {
        if path
            .last()
            .is_none_or(|last| last.distance_squared(point) > merge_distance_sq)
        {
            path.push(point);
        }
    }
    if closed {
        while path.len() > 1 && path[0].distance_squared(path[path.len() - 1]) <= merge_distance_sq
        {
            path.pop();
        }
    }
    path
}

/// Сколько хорд нужно дуге радиуса `radius` на `sweep` радиан, чтобы стрелка
/// хорды осталась в пределах [`ARC_TOLERANCE`].
pub(crate) fn arc_steps(radius: f32, sweep: f32) -> usize {
    let max_step = if radius > ARC_TOLERANCE {
        2.0 * (1.0 - ARC_TOLERANCE / radius).acos()
    } else {
        PI
    };
    ((sweep / max_step).ceil() as usize).clamp(1, MAX_ARC_STEPS)
}

/// Не видна ли щель butt-квадов на изломе в `turn` радиан у ленты полуширины
/// `radius`: её ширина `radius · |turn|` мельче [`ARC_TOLERANCE`]. Одно правило
/// на обе стороны излома круглой ленты — такому веер не кладётся
/// ([`MeshBuilder::push_join_fan`]), и такой же квады проходят общими
/// вершинами; разойдись они, вернулась бы либо волосяная щель, либо веер поверх
/// общих вершин.
fn join_gap_hidden(radius: f32, turn: f32) -> bool {
    radius * turn.abs() < ARC_TOLERANCE
}

/// Расстояние от точки до ломаной — мерка тестов на геометрию лент: ни одна
/// вершина не имеет права уйти от осевой дальше, чем обещает примитив. Одна
/// на тесты мешинга, дорог и рельсов.
#[cfg(test)]
pub(crate) fn distance_to_path(point: Vec2, path: &[Vec2]) -> f32 {
    path.windows(2)
        .map(|segment| {
            let span = segment[1] - segment[0];
            let t = (point - segment[0]).dot(span) / span.length_squared();
            point.distance(segment[0] + span * t.clamp(0.0, 1.0))
        })
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests;
