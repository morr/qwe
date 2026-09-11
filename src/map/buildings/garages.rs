//! Гаражные кооперативы: ГСК рисуется рядами боксов, а не одной коробкой.
//!
//! На снимке русского города ГСК ни с чем не спутать: сетка мелких боксов,
//! разрезанная проездами. В OSM он приезжает двумя разными способами, и до
//! сих пор оба рисовались неправильно:
//!
//! * **боксы по одному** (`building=garage`) — каждый со своим посевом, своим
//!   материалом кровли и своей фазой фактуры, так что ряд из двадцати боксов
//!   выходил конфетти из черепицы, битума и профлиста;
//! * **кооператив целиком** (`building=garages`) — одним контуром на всю
//!   территорию, и в Туле это пятна до 255 × 51 м, нарисованные как один
//!   гигантский ангар с зенитными фонарями.
//!
//! Здесь оба случая сводятся к одному: контуры сшиваются в **прогон**
//! ([`garage_runs`]), и весь прогон получает один посев — им выбираются
//! материал и цвет, и он же превращает двадцать домиков в одну ленту.
//!
//! **Геометрию же несёт не прогон, а кусок** ([`GarageRect`]): контур режется
//! на почти-прямоугольные куски ([`split_rings`]), и своя ось, своя сетка
//! ячеек и свой кусок кровли — у каждого из них. Так и надо: ось у прогона
//! бралась из `min_area_rect` по всем его точкам, а у буквы Г такой
//! прямоугольник пуст на 80 % — общая ось вставала наискось к стенам обоих
//! крыльев, проезды кооператива шли поперёк всей буквы, а пороги «лента или
//! кооператив» мерились по пустому прямоугольнику. Разрезанный контур отвечает
//! на всё это сам: ось куска параллельна его собственным стенам по построению.
//! Прогону остаются посев и право называться гаражным ([`group_reads_as_garage`]).
//!
//! Дальше рисует шейдер ([`super::material::RoofKind::GarageRow`] и
//! `GarageBlock`): шов на каждой границе бокса, а у кооператива ещё и тёмный
//! проезд на границе ряда. Метры остаются здесь: в вершину едут ячейки, и
//! шейдеру достаточно их дробной части. Ни одной новой вершины — только другие
//! значения в том же `ATTRIBUTE_ROOF`.

use std::collections::HashMap;

use bevy::prelude::*;

use super::material::building_seed;
use crate::map::meshing::min_area_rect;
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{BuildingUse, PolyArea};

/// Зазор, ближе которого два гаража считаются одним прогоном, м. Боксы обычно
/// стоят стена к стене, но контуры в OSM разведены на десятки сантиметров.
///
/// Зазор меряется **между контурами**, а не между их осевыми рамками, и это
/// не педантизм: лента ГСК идёт под произвольным углом, её осевая рамка втрое
/// шире её самой, и рамки соседних лент перекрываются насквозь через проезд.
/// На туламском ГСК у Косой Горы по рамкам в один прогон слипались десять
/// параллельных лент — прогон 350 × 72 м, чья ось расходилась с осями самих
/// лент на 7°, и швы вставали наискось к их стенам.
const JOIN_GAP: f32 = 2.0;
/// Лента короче этого прогоном не считается: гребёнка начинает читаться
/// примерно с четырёх боксов, а до того это просто пара сараев.
const ROW_MIN_LENGTH: f32 = 12.0;
/// И она обязана быть **лентой**, а не пятном: у квадратного пятна нет оси,
/// и поперечный шов на нём пошёл бы наугад.
const ROW_MIN_ASPECT: f32 = 2.2;
/// Пятно шириной от этого держит уже не ленту, а **ряды с проездом** между
/// ними — 12 м боксов спинами плюс проезд.
const BLOCK_MIN_WIDTH: f32 = 14.0;
/// И оно обязано быть кооперативом по площади, а не гаражом на две машины.
const BLOCK_MIN_AREA: f32 = 400.0;
/// Шаг бокса, м. Это **нижняя граница и цель, а не делитель**: сколько боксов
/// встанет на ленту, решает деление нацело, поэтому на своей ленте бокс шире
/// — зато их целое число и на торцах нет огрызка.
///
/// Нацело, а не «до ближайшего»: округление вверх делает бокс **уже** мерки, и
/// при длине чуть больше полутора боксов — на треть уже, 2.6 м вместо 3.9.
/// В такую ячейку не влезает ни машина, ни нарисованная над ней створка, и на
/// карте это читается как слишком частая гребёнка из дверей-щелей. Остаток же
/// делится поровну между **всеми** боксами этой ленты, а не достаётся
/// крайнему: ряд одинаковых ворот — это и есть то, чем ГСК читается.
///
/// Мерка тут не «примерно так на фотографии», а **машина, которую рисует сама
/// игра**: `cars::body::CarShape` — от хэтчбека 1.72 м до фургона 1.95 м в
/// ширину. Бокс обязан принять самую широкую с зазором на открытые двери
/// (полметра с каждой стороны) и простенком между воротами, отсюда
/// `1.95 + 2 × 0.55 + 0.4 ≈ 3.9`. Прежние 3.4 м были нижней границей
/// физически возможного — фургон входил впритык, — и на карте ряд читался
/// слишком частой гребёнкой.
pub(super) const BAY: f32 = 3.9;
/// Шаг рядов в кооперативе, м, той же природы: два ряда боксов спинами
/// (6 + 6) и проезд между парами (6). И делится ширина так же нацело: ряд уже
/// мерки — это проезд, в который не въехать.
const ROW_PITCH: f32 = 18.0;
/// Ячейка пространственного хеша при поиске соседей, м.
const CELL: f32 = 32.0;
/// Насколько плотно кусок обязан заполнять свой минимальный прямоугольник,
/// чтобы резать его дальше было незачем. Та же мерка, по которой [`roofs`]
/// решает, ложится ли на дом двускатная крыша, и почти тот же порог: у
/// прямоугольника заполнение единица, у буквы Г — от 0.2 до 0.6.
///
/// [`roofs`]: super::roofs
const RECT_FILL: f32 = 0.90;
/// Кусок мельче этого не вырезается: срезать у контура угол в пару метров —
/// это не «разрезать на прямоугольники», а накрошить стружки, каждая из
/// которых получит свою ось и свой шов.
const PIECE_MIN_AREA: f32 = 8.0;
/// И уже этого — тоже: стружка 20 × 1 м проходит и по площади, и по
/// заполнению (тонкий прямоугольник заполняет свою рамку идеально), а гаражом
/// шириной в метр быть не может. Мерка — машина: у`же самой узкой ей некуда
/// встать.
const PIECE_MIN_WIDTH: f32 = 3.0;
/// Сколько раз подряд можно резать. Гаражный контур в Туле — это буква Г или
/// гребёнка на три-четыре зуба; шесть разрезов с запасом покрывают и её, а
/// предел нужен на случай контура, который разрезами не улучшить.
const SPLIT_DEPTH_MAX: u32 = 6;
/// Косинус, начиная с которого стена считается идущей **вдоль** оси своего
/// куска, то есть фасадом с воротами. 45°: у куска, подогнанного под
/// собственные стены, они идут под 0° или 90°, и делить надо ровно посередине.
pub(super) const FACADE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Кусок гаражного контура: почти прямоугольник со своей осью и **своей
/// сеткой ячеек** — целое число боксов вдоль и рядов поперёк.
///
/// Ячейки, а не метры, — то же лекарство, что у стены (`layers::wall_frame`):
/// сетка в метрах не знает, где лента кончается, и на торце остаётся
/// отрезанный бокс. Здесь шаг подогнан под сам кусок, поэтому шов приходится
/// ровно на оба его торца, а проезд — на край пятна.
#[derive(Clone, Debug)]
pub(super) struct GarageRect {
    /// Угол куска, от которого считаются ячейки: минимум по обеим осям.
    pub(super) origin: Vec2,
    /// Вдоль куска — длинная ось его минимального прямоугольника, а значит и
    /// его собственных длинных стен.
    pub(super) axis: Vec2,
    /// Длина куска вдоль оси, м.
    pub(super) length: f32,
    /// Ширина куска поперёк оси, м.
    pub(super) width: f32,
    /// Шаг бокса на **этом** куске, м: длина, делённая на целое число боксов.
    pub(super) bay: f32,
    /// Шаг ряда поперёк, м: у кооператива ширина, делённая на целое число
    /// пар рядов; у ленты — вся её ширина, один ряд на ленту.
    pub(super) row: f32,
    /// Кооператив (`building=garages`, широкое пятно) — рисуется рядами с
    /// проездами; иначе одна лента боксов. Решается **по куску**: у буквы Г
    /// широким может быть одно крыло из двух.
    pub(super) block: bool,
    /// Гаража на этом куске **не рисуется вовсе**: ни гребёнки боксов на
    /// кровле, ни ворот на стенах — только профлист того же цвета, что у
    /// соседей по контуру.
    ///
    /// Так помечается обрезок разрезанного контура, который сам по себе
    /// гаражным не читается: зуб гребёнки в четыре метра, клин у крыла,
    /// огрызок между двумя лентами. Выдумывать ему ось неоткуда — у почти
    /// квадратного пятна её нет, — а выданная соседом разворачивает его
    /// гребёнку поперёк собственных стен и режет крайние ворота пополам.
    /// Целый контур в сшитом прогоне — случай другой: коробка 3 × 6 читается
    /// лентой вместе с соседями, и ось ей достаётся от них ([`GarageRect::take_grid`]).
    pub(super) plain: bool,
    /// Кольцо самого куска — им и кроется кровля: куски покрывают контур дома
    /// целиком и без нахлёста, так что резать ещё раз нечего.
    pub(super) ring: Vec<Vec2>,
}

/// Прогон гаражей глазами отрисовки: общий посев и куски, на которые разрезан
/// контур **этого** дома.
///
/// Прогон сшивает соседние контуры (`garage_runs`), но геометрию каждому
/// оставляет его собственную: общей у них только «одежда». Сшитая лента из
/// двадцати боксов — это двадцать прогонов с одним посевом, а не один прогон с
/// одной осью на всех; осью, посчитанной по конкатенации их колец, и вставали
/// швы наискось к стенам.
#[derive(Clone, Debug)]
pub(super) struct GarageRun {
    /// Посев прогона (минимум по боксам, поэтому не зависит от их порядка) —
    /// им выбираются цвет и разнобой тона.
    pub(super) seed: u32,
    /// Куски контура, непусто. Один кусок — значит резать было нечего.
    pub(super) rects: Vec<GarageRect>,
}

impl GarageRect {
    /// Сетка куска по его кольцу: ось от минимального прямоугольника, шаги —
    /// подгонкой под его длину и ширину. `None` — вырожденное кольцо.
    fn of(ring: Vec<Vec2>, cooperative: bool) -> Option<Self> {
        let rect = min_area_rect(&ring)?;
        // `min_area_rect` кладёт длинную сторону первой
        let axis = (rect[1] - rect[0]).try_normalize()?;
        let across = Vec2::new(-axis.y, axis.x);
        let length = (rect[1] - rect[0]).length();
        let width = (rect[2] - rect[1]).length();
        // угол прямоугольника: минимум по обеим осям, от него и считаются ячейки
        let low = |direction: Vec2| {
            rect.iter()
                .map(|at| at.dot(direction))
                .fold(f32::MAX, f32::min)
        };
        let block = is_block(cooperative, length, width);
        Some(Self {
            origin: axis * low(axis) + across * low(across),
            axis,
            length,
            width,
            // целое число боксов на длину: шаг подгоняется под кусок, а не
            // кусок под шаг, и на торцах не остаётся огрызка. Нацело, а не до
            // ближайшего: бокс уже мерки — это ячейка, в которую не влезает
            // ни машина, ни створка над ней (см. [`BAY`])
            bay: length / (length / BAY).floor().max(1.0),
            row: match block {
                // у кооператива целое число рядов: крайний ряд не разрезан
                // проездом посередине
                true => width / (width / ROW_PITCH).floor().max(1.0),
                // у ленты ряд один на всю ширину: поперечной координате
                // остаётся сказать только «внутри», и проездов на ней не бывает
                false => width.max(1e-3),
            },
            block,
            plain: false,
            ring,
        })
    }

    /// Кусок читается как гаражный сам по себе — лента из нескольких боксов
    /// или кооператив шириной в два ряда с проездом.
    fn reads_as_garage(&self) -> bool {
        self.block || is_row(self.length, self.width)
    }

    /// Тот же кусок на **чужой** сетке: кольцо своё, всё остальное соседа.
    ///
    /// Это для куска, который сам по себе гаражным не читается: у коробки
    /// 3.2 × 6 длинная ось идёт поперёк ленты, в которую она встала, и швы по
    /// ней пошли бы поперёк гребёнки, а ворота — на глухую стену между
    /// соседями. Своя ось есть только у того, кто сам по себе лента или пятно;
    /// остальные берут её у прогона, ради чего прогон и сшивается.
    fn take_grid(&mut self, grid: &GarageRect) {
        let ring = std::mem::take(&mut self.ring);
        *self = Self {
            ring,
            ..grid.clone()
        };
    }

    /// Расстояние от точки до **границы** куска. Им стена и находит свой
    /// кусок: куски покрывают контур целиком, поэтому середина всякой стены
    /// лежит на кольце ровно одного из них и до него расстояние ноль.
    ///
    /// Именно до кольца, а не до прямоугольника: прямоугольник у куска
    /// габаритный, и у буквы V рамка одного крыла накрывает часть другого.
    /// Тогда стена второго крыла оказывалась внутри обеих, оба расстояния
    /// выходили нулевыми, и крыло доставалось первому в списке — вместе с его
    /// осью, повёрнутой к этой стене поперёк. На картинке это ровно то, что
    /// видно: фасад с воротами на одном крыле и глухая стена там, где ворота
    /// должны быть, на другом.
    fn distance(&self, point: Vec2) -> f32 {
        let count = self.ring.len();
        (0..count)
            .map(|at| point_to_segment(point, self.ring[at], self.ring[(at + 1) % count]))
            .fold(f32::MAX, f32::min)
    }

    /// Идёт ли эта стена **вдоль** оси куска, то есть фасадом с воротами.
    /// Поперечная стена — торец: ворот на нём нет, в гараж въезжают с проезда.
    pub(super) fn faces_the_drive(&self, along: Vec2) -> bool {
        along.dot(self.axis).abs() >= FACADE_COS
    }
}

impl GarageRun {
    /// Кусок, которым дом читается: самый большой **из настоящих** — обрезок,
    /// на котором гаража не рисуется, материала дому не выбирает. Им и
    /// выбираются материал кровли (лента или кооператив) и номинальная ось;
    /// цвет и посев всё равно общие на прогон.
    pub(super) fn main(&self) -> &GarageRect {
        let widest = |plain: bool| {
            self.rects
                .iter()
                .filter(move |rect| rect.plain == plain)
                .max_by(|a, b| {
                    (a.length * a.width)
                        .partial_cmp(&(b.length * b.width))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        };
        widest(false)
            .or_else(|| widest(true))
            .expect("прогон без кусков не создаётся")
    }

    /// Кусок, которому принадлежит эта точка, — ближайший.
    pub(super) fn holder(&self, point: Vec2) -> &GarageRect {
        self.rects
            .iter()
            .min_by(|a, b| {
                a.distance(point)
                    .partial_cmp(&b.distance(point))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("прогон без кусков не создаётся")
    }
}

/// Прогоны по контурам: индекс здания → прогон, в который оно вошло. Здания
/// вне прогонов (одиночный гараж, сарай, пара боксов) в карту не попадают и
/// рисуются по-старому.
pub(super) fn garage_runs(buildings: &[PolyArea]) -> HashMap<usize, GarageRun> {
    let members: Vec<usize> = buildings
        .iter()
        .enumerate()
        .filter(|(_, building)| {
            matches!(
                building.building_use,
                BuildingUse::Garage | BuildingUse::GarageBlock
            )
        })
        .map(|(index, _)| index)
        .collect();
    let mut runs = HashMap::new();
    if members.is_empty() {
        return runs;
    }
    let boxes: Vec<Aabb> = members
        .iter()
        .map(|&at| aabb(&buildings[at].outer))
        .collect();

    // Пространственный хеш: гараж мал, поэтому раскладывается в одну-две
    // ячейки, и пары ищутся внутри ячейки, а не по всему городу. Раздутый
    // на зазор контур попадает в каждую задетую ячейку, поэтому два
    // достаточно близких бокса гарантированно встретятся хотя бы в одной.
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (slot, box_) in boxes.iter().enumerate() {
        let low = ((box_.min - JOIN_GAP) / CELL).floor();
        let high = ((box_.max + JOIN_GAP) / CELL).floor();
        for x in low.x as i32..=high.x as i32 {
            for y in low.y as i32..=high.y as i32 {
                cells.entry((x, y)).or_default().push(slot);
            }
        }
    }
    let mut union = Union::new(members.len());
    for slots in cells.values() {
        for (at, &a) in slots.iter().enumerate() {
            for &b in &slots[at + 1..] {
                // рамка — только отсев: она у диагональной ленты втрое шире
                // самой ленты, и через проезд задевает соседнюю. Решает
                // расстояние между контурами
                if boxes[a].near(&boxes[b])
                    && rings_near(&buildings[members[a]].outer, &buildings[members[b]].outer)
                {
                    union.join(a, b);
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for slot in 0..members.len() {
        groups.entry(union.root(slot)).or_default().push(slot);
    }
    for group in groups.values() {
        // проезды рисуются только там, где картограф сказал «кооператив»:
        // на большом сарае они были бы выдумкой
        let cooperative = group
            .iter()
            .all(|&slot| buildings[members[slot]].building_use == BuildingUse::GarageBlock);
        // каждый контур режется по себе: куску нужны его собственные стены,
        // а не общая рамка прогона
        let cut: Vec<(usize, Vec<GarageRect>)> = group
            .iter()
            .map(|&slot| {
                let building = &buildings[members[slot]];
                let rects = split_rings(&building.outer, building.holes.is_empty())
                    .into_iter()
                    .filter_map(|piece| GarageRect::of(piece, cooperative))
                    .collect();
                (slot, rects)
            })
            .collect();
        // Гаражным прогон читается двумя способами, и нужны оба. **Целиком** —
        // так читается сшитая лента, каждый бокс которой сам по себе просто
        // коробка 6 × 3. **Хотя бы одним куском** — так читается буква Г,
        // общая рамка которой пуста на 80 % и по ней не проходит ни один порог.
        let points: Vec<Vec2> = group
            .iter()
            .flat_map(|&slot| buildings[members[slot]].outer.iter().copied())
            .collect();
        let reads = group_reads_as_garage(&points, cooperative)
            || cut
                .iter()
                .any(|(_, rects)| rects.iter().any(GarageRect::reads_as_garage));
        if !reads {
            continue;
        }
        let seed = group
            .iter()
            .map(|&slot| building_seed(&buildings[members[slot]]))
            .min()
            .unwrap_or(0);
        // сетка прогона целиком — её берёт кусок, который сам гаражным не
        // читается. Кольцо у неё ничего не значит (это конкатенация колец
        // группы, а не контур), и в отрисовку она попадает только сеткой
        let common = GarageRect::of(points, cooperative).filter(GarageRect::reads_as_garage);
        for (slot, mut rects) in cut {
            if rects.is_empty() {
                continue;
            }
            // Кусок, который сам гаражным не читается, решается по тому,
            // **разрезан ли контур**. Целый — это коробка 3 × 6 в сшитом
            // прогоне: лентой она читается вместе с соседями, и ось ей
            // достаётся от них, ради чего прогон и сшивается. Обрезок
            // разрезанного — зуб гребёнки, клин у крыла: гаража на нём не
            // рисуется вовсе ([`GarageRect::plain`]).
            let carved = rects.len() > 1;
            let grid = largest(&rects).or(common.clone());
            for rect in &mut rects {
                if rect.reads_as_garage() {
                    continue;
                }
                match (carved, &grid) {
                    (false, Some(grid)) => rect.take_grid(grid),
                    _ => rect.plain = true,
                }
            }
            runs.insert(members[slot], GarageRun { seed, rects });
        }
    }
    runs
}

/// Самый большой из кусков, которые читаются гаражными сами по себе, — им и
/// делится ось с обрезками того же контура.
fn largest(rects: &[GarageRect]) -> Option<GarageRect> {
    rects
        .iter()
        .filter(|rect| rect.reads_as_garage())
        .max_by(|a, b| {
            (a.length * a.width)
                .partial_cmp(&(b.length * b.width))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Широкое пятно кооператива — ряды с проездами: ширины хватает на два ряда
/// боксов спинами **и** проезд между ними. Проверка стоит первой и только на
/// множественном теге: проезды поперёк `building=shed` были бы выдумкой.
fn is_block(cooperative: bool, length: f32, width: f32) -> bool {
    cooperative && width >= BLOCK_MIN_WIDTH && length * width >= BLOCK_MIN_AREA
}

/// Лента боксов: достаточно длинная, чтобы гребёнка читалась, и достаточно
/// вытянутая, чтобы у неё вообще была ось.
fn is_row(length: f32, width: f32) -> bool {
    length >= ROW_MIN_LENGTH && length >= width * ROW_MIN_ASPECT
}

/// Читается ли как гаражное **пятно целиком** — та же мерка, что у куска, но
/// по набору точек: у сшитого прогона своего кольца нет, а сшивают его как раз
/// затем, чтобы двадцать коробок 6 × 3 прочитались одной лентой.
fn group_reads_as_garage(points: &[Vec2], cooperative: bool) -> bool {
    let Some(rect) = min_area_rect(points) else {
        return false;
    };
    let length = (rect[1] - rect[0]).length();
    let width = (rect[2] - rect[1]).length();
    is_block(cooperative, length, width) || is_row(length, width)
}

/// Контур, разрезанный на почти-прямоугольные куски. Куски покрывают его
/// целиком и без нахлёста — разрез идёт хордой по самому кольцу, — поэтому
/// кровля кладётся по куску за раз и клеить ничего не надо.
///
/// `whole` — «резать нельзя»: у контура с дворами резать пришлось бы и дыры,
/// а дыра, попавшая не в тот кусок, — это двор, накрытый кровлей. Гаражей с
/// дворами в данных единицы, и они остаются одним куском, как были.
fn split_rings(ring: &[Vec2], whole: bool) -> Vec<Vec<Vec2>> {
    let mut pieces = Vec::new();
    match whole {
        true => split_into(ring.to_vec(), 0, &mut pieces),
        false => pieces.push(ring.to_vec()),
    }
    pieces
}

/// Рекурсия разреза: пока кусок заполняет свой прямоугольник хуже
/// [`RECT_FILL`] и находится разрез, который это улучшает.
fn split_into(ring: Vec<Vec2>, depth: u32, pieces: &mut Vec<Vec<Vec2>>) {
    let fill = fill_of(&ring);
    // у четырёхугольника вогнутой вершины не бывает, резать его не от чего
    if depth >= SPLIT_DEPTH_MAX || ring.len() < 5 || fill >= RECT_FILL {
        pieces.push(ring);
        return;
    }
    let Some((score, near, far)) = best_cut(&ring) else {
        pieces.push(ring);
        return;
    };
    // разрез, который ничего не улучшил, — это лишний шов и лишняя ось
    if score <= fill + 1e-3 {
        pieces.push(ring);
        return;
    }
    split_into(near, depth + 1, pieces);
    split_into(far, depth + 1, pieces);
}

/// Лучший разрез кольца: из каждой **вогнутой** вершины продолжается внутрь
/// каждое из двух её рёбер, и выигрывает тот разрез, после которого куски в
/// среднем (по площади) плотнее заполняют свои прямоугольники.
///
/// Вогнутая вершина, а не произвольная линия, — потому что резать надо ровно
/// там, где контур перестал быть прямоугольником, и по направлению **его
/// собственной стены**: только так ось куска окажется параллельна его стенам,
/// ради чего всё и затевается. У буквы Г такая вершина одна, и разрез из неё
/// делит её на два крыла.
fn best_cut(ring: &[Vec2]) -> Option<(f32, Vec<Vec2>, Vec<Vec2>)> {
    let count = ring.len();
    let area = ring_area(ring);
    if area < PIECE_MIN_AREA * 2.0 {
        return None;
    }
    let orientation = signed_ring_area(ring).signum();
    let mut best: Option<(f32, Vec<Vec2>, Vec<Vec2>)> = None;
    for at in 0..count {
        let prev = ring[(at + count - 1) % count];
        let here = ring[at];
        let next = ring[(at + 1) % count];
        let (back, ahead) = (here - prev, next - here);
        if back.perp_dot(ahead) * orientation >= 0.0 {
            continue;
        }
        // продолжение входящего ребра и продолжение исходящего назад — обе
        // прямые идут внутрь контура из этой вершины
        for direction in [back, -ahead] {
            let Some(direction) = direction.try_normalize() else {
                continue;
            };
            let Some((near, far)) = cut_at(ring, at, direction) else {
                continue;
            };
            let (near_area, far_area) = (ring_area(&near), ring_area(&far));
            if near_area < PIECE_MIN_AREA || far_area < PIECE_MIN_AREA {
                continue;
            }
            // стружка проходит и по площади, и по заполнению, но гаражом не
            // бывает — и получила бы свою ось, свою гребёнку и свои ворота
            if [&near, &far]
                .iter()
                .any(|piece| sides_of(piece).is_none_or(|(_, width)| width < PIECE_MIN_WIDTH))
            {
                continue;
            }
            let score = (fill_of(&near) * near_area + fill_of(&far) * far_area) / area;
            if best.as_ref().is_none_or(|(top, ..)| score > *top) {
                best = Some((score, near, far));
            }
        }
    }
    best
}

/// Разрез кольца хордой: луч из вершины `at` по `direction` до **ближайшего**
/// ребра, и кольцо распадается на два по этой хорде.
///
/// Ближайшего — в этом всё дело: бесконечная прямая у гребёнки прошла бы
/// насквозь через остальные зубья и оставила бы кусок, слепленный из
/// нескольких кусков перемычками нулевой ширины. Хорда же делит простое
/// кольцо ровно на два простых кольца.
fn cut_at(ring: &[Vec2], at: usize, direction: Vec2) -> Option<(Vec<Vec2>, Vec<Vec2>)> {
    let count = ring.len();
    let from = ring[at];
    let mut hit: Option<(f32, usize)> = None;
    for edge in 0..count {
        // рёбра самой вершины луч задевает в её же точке
        if edge == at || (edge + 1) % count == at {
            continue;
        }
        let (a, b) = (ring[edge], ring[(edge + 1) % count]);
        let span = b - a;
        let den = direction.perp_dot(span);
        if den.abs() < 1e-9 {
            continue;
        }
        let offset = a - from;
        let along = offset.perp_dot(span) / den;
        let across = offset.perp_dot(direction) / den;
        if along <= 1e-4 || !(-1e-6..=1.0 + 1e-6).contains(&across) {
            continue;
        }
        if hit.as_ref().is_none_or(|(nearest, _)| along < *nearest) {
            hit = Some((along, edge));
        }
    }
    let (along, edge) = hit?;
    let point = from + direction * along;
    let mut near: Vec<Vec2> = (0..=(edge + count - at) % count)
        .map(|step| ring[(at + step) % count])
        .collect();
    near.push(point);
    // дальний кусок идёт от точки попадания по кольцу **до самой вершины
    // разреза включительно**: она принадлежит обоим кускам — это её хорда.
    // Без неё кусок замыкался от предыдущей вершины прямо на точку, и
    // треугольник между ними не доставался никому: на карте у каждого зуба
    // гребёнки получалась дыра цвета земли
    let mut far = vec![point];
    far.extend((0..=(at + count - edge - 1) % count).map(|step| ring[(edge + 1 + step) % count]));
    let (near, far) = (dedup(near), dedup(far));
    (near.len() >= 3 && far.len() >= 3).then_some((near, far))
}

/// Кольцо без повторяющихся подряд точек: хорда, попавшая в вершину, иначе
/// оставила бы в куске нулевое ребро.
fn dedup(mut ring: Vec<Vec2>) -> Vec<Vec2> {
    ring.dedup_by(|a, b| a.distance(*b) < 1e-4);
    if ring.len() > 1 && ring[0].distance(ring[ring.len() - 1]) < 1e-4 {
        ring.pop();
    }
    ring
}

/// Стороны минимального прямоугольника кольца: длинная и короткая.
fn sides_of(ring: &[Vec2]) -> Option<(f32, f32)> {
    let rect = min_area_rect(ring)?;
    Some(((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length()))
}

/// Насколько плотно кольцо заполняет свой минимальный прямоугольник: единица
/// у прямоугольника, доли у буквы Г.
fn fill_of(ring: &[Vec2]) -> f32 {
    let Some((length, width)) = sides_of(ring) else {
        return 0.0;
    };
    match length * width > 1e-6 {
        true => ring_area(ring) / (length * width),
        false => 0.0,
    }
}

fn ring_area(ring: &[Vec2]) -> f32 {
    signed_ring_area(ring).abs()
}

/// Расстояние между двумя кольцами не больше [`JOIN_GAP`]: пары рёбер, без
/// проверки на вложенность — гаражи друг в друга не вкладываются.
fn rings_near(a: &[Vec2], b: &[Vec2]) -> bool {
    a.iter().enumerate().any(|(at, &a0)| {
        let a1 = a[(at + 1) % a.len()];
        b.iter().enumerate().any(|(to, &b0)| {
            let b1 = b[(to + 1) % b.len()];
            segments_distance(a0, a1, b0, b1) <= JOIN_GAP
        })
    })
}

/// Расстояние между отрезками. Пересечение считать не нужно: контуры соседних
/// гаражей не пересекаются, а касание даёт ноль и так — через концы.
fn segments_distance(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f32 {
    point_to_segment(a0, b0, b1)
        .min(point_to_segment(a1, b0, b1))
        .min(point_to_segment(b0, a0, a1))
        .min(point_to_segment(b1, a0, a1))
}

pub(super) fn point_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let edge = b - a;
    let length2 = edge.length_squared();
    if length2 < 1e-12 {
        return point.distance(a);
    }
    let at = ((point - a).dot(edge) / length2).clamp(0.0, 1.0);
    point.distance(a + edge * at)
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec2,
    max: Vec2,
}

impl Aabb {
    /// Зазор между коробками не больше [`JOIN_GAP`] по обеим осям.
    fn near(&self, other: &Aabb) -> bool {
        self.min.x - JOIN_GAP <= other.max.x
            && other.min.x - JOIN_GAP <= self.max.x
            && self.min.y - JOIN_GAP <= other.max.y
            && other.min.y - JOIN_GAP <= self.max.y
    }
}

fn aabb(ring: &[Vec2]) -> Aabb {
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for point in ring {
        min = min.min(*point);
        max = max.max(*point);
    }
    Aabb { min, max }
}

/// Система непересекающихся множеств со сжатием путей — боксы сшиваются
/// попарно, а прогон нужен целиком.
struct Union {
    parent: Vec<usize>,
}

impl Union {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn root(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.parent[a] = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::AreaKind;

    fn garage_of(at: Vec2, size: Vec2, use_: BuildingUse) -> PolyArea {
        PolyArea {
            outer: vec![
                at,
                at + Vec2::new(size.x, 0.0),
                at + size,
                at + Vec2::new(0.0, size.y),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: use_,
            height: None,
            entrances: Vec::new(),
        }
    }

    fn garage(at: Vec2, size: Vec2) -> PolyArea {
        garage_of(at, size, BuildingUse::Garage)
    }

    /// Шесть боксов стена к стене — один прогон с осью вдоль ленты.
    #[test]
    fn adjacent_boxes_become_one_run() {
        let boxes: Vec<PolyArea> = (0..6)
            .map(|at| garage(Vec2::new(at as f32 * 3.3, 0.0), Vec2::new(3.2, 6.0)))
            .collect();
        let runs = garage_runs(&boxes);
        assert_eq!(runs.len(), 6);
        let first = runs[&0].main();
        for at in 1..6 {
            assert_eq!(runs[&at].seed, runs[&0].seed);
            assert!((runs[&at].main().axis - first.axis).length() < 1e-5);
        }
        // ось у коробки 3.2 × 6 своя идёт **поперёк** ленты: сама по себе она
        // не лента, а коробка, и ось ей достаётся от прогона
        assert!(first.axis.x.abs() > 0.99, "{}", first.axis);
        assert!(!first.block);
    }

    /// Стена вдоль прогона — фасад с воротами, поперечная — глухой торец.
    /// До разреза этого сказать было нельзя: облицовка выбирается на дом
    /// целиком, и створки вставали на всех четырёх стенах бокса, а на углу
    /// упирались друг в друга.
    #[test]
    fn gates_go_on_the_wall_along_the_run() {
        let runs = garage_runs(&[garage_of(
            Vec2::ZERO,
            Vec2::new(60.0, 7.0),
            BuildingUse::GarageBlock,
        )]);
        let rect = runs[&0].main();
        assert!(rect.faces_the_drive(Vec2::X), "длинная стена — фасад");
        assert!(!rect.faces_the_drive(Vec2::Y), "торец без ворот");
    }

    /// Буква Г режется на два крыла, и ось каждого идёт вдоль его собственных
    /// стен. Пока ось была одна на весь контур, она бралась из минимального
    /// прямоугольника буквы — пустого на 80 %, — и швы вставали наискось к
    /// стенам обоих крыльев сразу.
    #[test]
    fn an_l_shape_is_cut_into_wings() {
        let wing = 60.0;
        let deep = 8.0;
        let bent = PolyArea {
            outer: vec![
                Vec2::ZERO,
                Vec2::new(wing, 0.0),
                Vec2::new(wing, deep),
                Vec2::new(deep, deep),
                Vec2::new(deep, wing),
                Vec2::new(0.0, wing),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::GarageBlock,
            height: None,
            entrances: Vec::new(),
        };
        let runs = garage_runs(&[bent]);
        let run = &runs[&0];
        assert_eq!(run.rects.len(), 2, "у буквы Г два крыла");
        for rect in &run.rects {
            assert!(
                rect.length >= 50.0 && rect.width <= deep + 1e-3,
                "крыло {} × {}",
                rect.length,
                rect.width
            );
            assert!(!rect.block, "крыло шириной 8 м не кооператив");
        }
        // каждое крыло досталось своей оси: одно вдоль X, другое вдоль Y
        let along_x = run.rects.iter().filter(|r| r.axis.x.abs() > 0.99).count();
        assert_eq!(along_x, 1, "оси крыльев расходятся на прямой угол");
        // и стена находит своё крыло: середина южной стены длинного крыла
        let south = run.holder(Vec2::new(wing * 0.75, 0.0));
        assert!(south.faces_the_drive(Vec2::X), "фасад длинного крыла");
    }

    /// То же на **настоящем** контуре из Тулы — гребёнке ГСК у Косой Горы
    /// (`way/1469565935`, 17 вершин, 1027 м²). Синтетическая буква Ш режется
    /// красиво, а этот зуб шириной в четыре метра — та фигура, на которой в
    /// кровле и появилась дыра цвета земли.
    #[test]
    fn the_pieces_cover_a_real_comb_from_the_city() {
        let comb = PolyArea {
            outer: vec![
                Vec2::new(1553.59, 1864.98),
                Vec2::new(1597.15, 1913.04),
                Vec2::new(1595.30, 1914.57),
                Vec2::new(1607.81, 1930.06),
                Vec2::new(1613.30, 1925.75),
                Vec2::new(1608.07, 1919.40),
                Vec2::new(1625.75, 1904.77),
                Vec2::new(1633.66, 1895.69),
                Vec2::new(1626.94, 1889.81),
                Vec2::new(1623.89, 1892.86),
                Vec2::new(1619.72, 1888.03),
                Vec2::new(1609.99, 1897.29),
                Vec2::new(1615.35, 1902.92),
                Vec2::new(1607.48, 1909.08),
                Vec2::new(1596.67, 1896.77),
                Vec2::new(1593.66, 1899.43),
                Vec2::new(1558.08, 1860.11),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::GarageBlock,
            height: None,
            entrances: Vec::new(),
        };
        let whole = ring_area(&comb.outer);
        let runs = garage_runs(&[comb]);
        let cut: f32 = runs[&0]
            .rects
            .iter()
            .map(|rect| ring_area(&rect.ring))
            .sum();
        // допуск относительный: координаты тут городские (~1600 м), и площадь
        // по формуле шнурков складывает произведения около 2.5 млн — у `f32`
        // это доли метра шума, а не потерянный кусок
        assert!(
            (cut - whole).abs() < whole * 1e-3,
            "куски дают {cut} м² против {whole} м² контура"
        );
    }

    /// Куски покрывают контур целиком и без нахлёста — иначе кровля,
    /// уложенная по куску за раз, была бы дырявой.
    #[test]
    fn the_pieces_cover_the_whole_outline() {
        let comb = PolyArea {
            outer: vec![
                Vec2::ZERO,
                Vec2::new(70.0, 0.0),
                Vec2::new(70.0, 9.0),
                Vec2::new(44.0, 9.0),
                Vec2::new(44.0, 40.0),
                Vec2::new(35.0, 40.0),
                Vec2::new(35.0, 9.0),
                Vec2::new(0.0, 9.0),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::GarageBlock,
            height: None,
            entrances: Vec::new(),
        };
        let whole = ring_area(&comb.outer);
        let runs = garage_runs(&[comb]);
        let cut: f32 = runs[&0]
            .rects
            .iter()
            .map(|rect| ring_area(&rect.ring))
            .sum();
        assert!((cut - whole).abs() < 1e-2, "{cut} против {whole}");
        assert!(runs[&0].rects.len() > 1, "гребёнку есть от чего резать");
    }

    /// Цельный `building=garages` лентой — тот же прогон, просто уже сшитый
    /// картографом; проездов на нём не рисуем, он их шириной не держит.
    #[test]
    fn a_single_long_outline_is_a_run_by_itself() {
        let runs = garage_runs(&[garage_of(
            Vec2::ZERO,
            Vec2::new(40.0, 6.0),
            BuildingUse::GarageBlock,
        )]);
        assert_eq!(runs.len(), 1);
        assert!(!runs[&0].main().block);
    }

    /// Пятно кооператива — ряды с проездами.
    #[test]
    fn a_wide_cooperative_outline_is_a_block() {
        let runs = garage_runs(&[garage_of(
            Vec2::ZERO,
            Vec2::new(120.0, 50.0),
            BuildingUse::GarageBlock,
        )]);
        assert!(runs[&0].main().block);
    }

    /// Такое же пятно, но это сарай: проезды поперёк сарая были бы выдумкой.
    #[test]
    fn a_big_shed_is_not_a_cooperative() {
        let runs = garage_runs(&[garage(Vec2::ZERO, Vec2::new(120.0, 50.0))]);
        assert!(runs.get(&0).is_none_or(|run| !run.main().block));
    }

    /// Одинокий гараж и пара сараев рядом рисуются по-старому.
    #[test]
    fn a_lone_garage_is_not_a_run() {
        let two = vec![
            garage(Vec2::ZERO, Vec2::new(3.2, 6.0)),
            garage(Vec2::new(3.3, 0.0), Vec2::new(3.2, 6.0)),
        ];
        assert!(garage_runs(&two).is_empty());
    }

    /// Два кооператива в разных концах города не сшиваются в один.
    #[test]
    fn distant_runs_stay_apart() {
        let mut boxes: Vec<PolyArea> = (0..6)
            .map(|at| garage(Vec2::new(at as f32 * 3.3, 0.0), Vec2::new(3.2, 6.0)))
            .collect();
        boxes.extend(
            (0..6).map(|at| garage(Vec2::new(at as f32 * 3.3, 400.0), Vec2::new(3.2, 6.0))),
        );
        let runs = garage_runs(&boxes);
        assert_eq!(runs.len(), 12);
        assert_ne!(runs[&0].seed, runs[&6].seed);
    }

    /// Координаты точки в ячейках куска — то же, что кладёт в вершину
    /// `layers::garage_frame`.
    fn cell(rect: &GarageRect, point: Vec2) -> Vec2 {
        let offset = point - rect.origin;
        let across = Vec2::new(-rect.axis.y, rect.axis.x);
        Vec2::new(
            offset.dot(rect.axis) / rect.bay,
            offset.dot(across) / rect.row,
        )
    }

    fn whole(value: f32) -> bool {
        (value - value.round()).abs() < 1e-3
    }

    /// На ленте целое число боксов: оба её торца приходятся на шов, поэтому
    /// обрезанного бокса на конце не бывает. Шаг для этого подгоняется под
    /// ленту и лежит около [`BAY`], а не равен ему.
    #[test]
    fn a_run_holds_a_whole_number_of_bays() {
        let start = Vec2::new(137.4, -58.1);
        let runs = garage_runs(&[garage(start, Vec2::new(40.0, 6.0))]);
        let rect = runs[&0].main();
        assert!(whole(cell(rect, start).x), "{}", cell(rect, start).x);
        let far = cell(rect, start + Vec2::new(40.0, 0.0)).x;
        assert!(whole(far), "{far}");
        assert_eq!(far.round(), 10.0, "40 м это 10 боксов по 4.0");
        assert!((rect.bay - BAY).abs() < 0.4, "шаг {}", rect.bay);
        // бокс обязан принять самую широкую машину игры с зазором на двери
        assert!(rect.bay > 1.95 + 1.0, "фургон должен войти: {}", rect.bay);
    }

    /// Бокс никогда не у`же мерки: остаток длины раскидывается поровну по
    /// **всем** боксам ленты, а не режет каждый из них.
    ///
    /// При округлении до ближайшего лента в 22 м получала шесть боксов по
    /// 3.67 м, а в 5.9 м — два по 2.95: в такую ячейку не встают ни машина, ни
    /// нарисованная над ней створка, и на карте это читается гребёнкой из
    /// дверей-щелей. Делением нацело те же ленты дают пять боксов по 4.4 и
    /// один на 5.9.
    #[test]
    fn a_bay_is_never_narrower_than_a_car_needs() {
        for length in [12.0_f32, 15.0, 22.0, 27.0, 40.0, 74.9, 258.5] {
            let runs = garage_runs(&[garage_of(
                Vec2::ZERO,
                Vec2::new(length, 5.0),
                BuildingUse::GarageBlock,
            )]);
            let rect = runs[&0].main();
            assert!(
                rect.bay >= BAY - 1e-3,
                "лента {length} м: бокс {}",
                rect.bay
            );
            assert!(
                rect.bay < BAY * 2.0,
                "лента {length} м: бокс {} — это уже два бокса в одном",
                rect.bay
            );
        }
    }

    /// А у кооператива целое число рядов: крайний ряд не разрезан проездом
    /// посередине.
    #[test]
    fn a_block_holds_a_whole_number_of_rows() {
        let start = Vec2::new(-311.7, 92.3);
        let runs = garage_runs(&[garage_of(
            start,
            Vec2::new(120.0, 54.0),
            BuildingUse::GarageBlock,
        )]);
        let rect = runs[&0].main();
        assert!(rect.block);
        assert!(whole(cell(rect, start).y));
        let far = cell(rect, start + Vec2::new(0.0, 54.0)).y;
        assert!(whole(far), "{far}");
        assert_eq!(far.round(), 3.0, "54 м это три пары рядов по 18");
    }

    /// Две параллельные ленты через проезд — **разные** прогоны, у каждой своя
    /// ось. Осевые рамки диагональной ленты перекрываются через проезд
    /// насквозь, и по ним они слипались в один прогон 350 × 72 м, чья ось
    /// расходилась с их собственными на 7°: швы вставали наискось к стенам, а
    /// пятно вдобавок проходило по ширине в кооператив и получало проезды
    /// поперёк всей пачки.
    #[test]
    fn parallel_ribbons_across_a_drive_stay_apart() {
        let along = Vec2::new(1.0, 1.0).normalize();
        let across = Vec2::new(-along.y, along.x);
        let ribbon = |offset: Vec2| PolyArea {
            outer: vec![
                offset,
                offset + along * 60.0,
                offset + along * 60.0 + across * 7.0,
                offset + across * 7.0,
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::GarageBlock,
            height: None,
            entrances: Vec::new(),
        };
        // вторая лента сдвинута поперёк на проезд и вдоль на половину длины —
        // так их осевые рамки и перекрываются
        let runs = garage_runs(&[ribbon(Vec2::ZERO), ribbon(across * 19.0 + along * 30.0)]);
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[&0].seed, runs[&1].seed);
        for run in runs.values() {
            let rect = run.main();
            assert!(!rect.block, "лента шириной 7 м не кооператив");
            assert!(
                rect.axis.perp_dot(along).abs() < 1e-3,
                "ось {} не вдоль ленты",
                rect.axis
            );
        }
    }
}
