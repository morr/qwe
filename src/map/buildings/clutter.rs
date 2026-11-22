//! Оборудование на кровле: машинные помещения лифтов, вентшахты, блоки
//! кондиционеров, зенитные фонари промышленных корпусов, трубы на коньках.
//!
//! Ровная крыша с воздуха не бывает пустой, и именно эта мелочь — коробки со
//! своими короткими тенями — делает снимок снимком, а не заливкой. Ставится
//! всё детерминированно: ГПСЧ засеян тем же посевом от первой вершины
//! контура, что выбирает материал кровли ([`super::material`]), так что
//! переключение режима высот или зума не переставляет оборудование.
//!
//! Что где стоит — решает материал кровли, а не назначение: мягкая плоская
//! (битум, гравий, мембрана) несёт машинное помещение и вентшахты, профлист
//! большого корпуса — ленты зенитных фонарей, скатная — трубу на коньке.
//! Разметка идёт в раме дома (длинная ось контура), с отступом от края, и
//! каждая коробка проверяется дважды: на попадание в контур — у Г-образного
//! дома прямоугольник рамы торчит наружу — и на свободное место, потому что
//! места берутся из ГПСЧ и две коробки иначе садятся одна на другую.
//!
//! Видно всё это только вблизи: [`super::BuildingZoomBucket`] снимает
//! оборудование целиком, когда метр кровли становится мельче пары пикселей —
//! иначе субпиксельные коробки мерцают при панораме. Снимает шейдером, а не
//! пересборкой: [`push_items`] метит вершины оборудования
//! (`MeshBuilder::set_clutter`), и вершинный шейдер кровель схлопывает их.

use bevy::color::Mix;
use bevy::prelude::*;

use super::Lean;
use super::classify::{BuildingClass, BuildingKind, classify};
use super::layers::{silhouette_edges, wall_colors};
use super::material::{RoofKind, RoofLook};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::PolyArea;
use crate::map::osm::model::{point_in_area, signed_ring_area};
use crate::map::seed::Lcg;
use crate::map::shadow;
use crate::map::shadow_dir;

mod placement;

use self::placement::{Frame, clear, fit, place, rect, shadow_reach};

#[cfg(test)]
mod tests;

/// Сколько мест перебрать, прежде чем отказаться от коробки. Одна попытка
/// на узком корпусе почти всегда промахивалась: машинное помещение 5 × 3.5 м
/// целиком укладывается в двенадцатиметровый дом лишь в узкой полосе. Шесть
/// хватало, пока место проверялось только на попадание в контур; с проверкой
/// на занятость ([`clear`]) десятая вентшахта на плотной кровле отбрасывается
/// куда чаще, и две попытки сверху возвращают её.
const PLACE_TRIES: usize = 8;

/// Машинное помещение лифта и выход на кровлю: одна коробка на дом крупнее
/// этого пятна, вторая — на дом вчетверо крупнее.
const PENTHOUSE_MIN_AREA: f32 = 400.0;
const PENTHOUSE_SECOND_AREA: f32 = 1600.0;
const PENTHOUSE_SIZE: Vec2 = Vec2::new(5.0, 3.5);
const PENTHOUSE_HEIGHT: f32 = 3.0;

/// Вентшахты и дефлекторы: одна на столько квадратов кровли.
const VENT_AREA_PER: f32 = 300.0;
const VENT_MAX: usize = 10;
const VENT_SIZE: Vec2 = Vec2::new(1.1, 1.1);
const VENT_HEIGHT: f32 = 1.2;

/// Блоки кондиционеров — у торговых и казённых зданий, рядком.
const UNIT_AREA_PER: f32 = 500.0;
const UNIT_MAX: usize = 6;
const UNIT_SIZE: Vec2 = Vec2::new(1.8, 1.0);
const UNIT_HEIGHT: f32 = 0.9;

/// Зенитные фонари гипермаркета — **решётка**, а не лента: торговый зал в
/// девять тысяч квадратов освещается сверху, и на каждом аэрофото кровля
/// коробки размечена правильной сеткой светлых квадратов. Это и есть то, по
/// чему гипермаркет узнают сверху раньше вывески.
///
/// Шаг — по колоннам каркаса: фонарь на ячейку сетки. Предел нужен не ради
/// вида, а ради вершин: у ТРЦ «Макси» (64 тыс. м²) без него вышло бы под
/// четыре сотни коробок в одном меше.
const GRID_SKYLIGHT_SIDE: f32 = 2.8;
const GRID_SKYLIGHT_PITCH: f32 = 13.0;
const GRID_SKYLIGHT_HEIGHT: f32 = 0.6;
/// Предел стоит не ради вида, а ради вершин, и взят он замером: фонарь это
/// шесть четырёхугольников с тенью, то есть около двух десятков вершин, так
/// что три сотни на здание — порядка семи тысяч, доли процента слоя зданий
/// (790 тыс. на Туле), и крупноформатных домов в городе пятнадцать.
/// При ста двадцати шаг на ТРЦ «Макси» растягивался до тридцати пяти метров, и
/// решётка выходила заметно реже, чем на аэрофото.
const GRID_SKYLIGHT_MAX: usize = 300;
/// Сколько раз растягивать шаг, подгоняя решётку под предел ([`grid_pitch`]), и
/// на сколько как минимум за раз. Корень из превышения сходится за две-три
/// итерации; пол на множителе нужен затем, чтобы превышение в доли процента не
/// крутило цикл вхолостую.
const GRID_PITCH_TRIES: usize = 6;
const GRID_PITCH_MIN_STEP: f32 = 1.02;

/// Блок приточной установки на кровле коробки: не бытовой кондиционер, а
/// агрегат в человеческий рост. На фотографиях они стоят **группой** у одного
/// края — там, где внизу зал, а не склад, — и группа эта читается сверху
/// отдельным пятном.
const PLANT_SIZE: Vec2 = Vec2::new(3.6, 2.2);
const PLANT_HEIGHT: f32 = 1.8;
const PLANT_GAP: f32 = 1.0;
const PLANT_AREA_PER: f32 = 3000.0;
const PLANT_MAX: usize = 5;

/// Вентшахт на кровле гипермаркета больше, чем на любой другой: предел
/// [`VENT_MAX`] в десять штук рассчитан на дом, а не на гектар кровли, и на
/// «Магните» он давал десяток точек на поле, где их должно быть несколько
/// десятков.
const VENT_MAX_BIG_BOX: usize = 26;

/// Зенитные фонари: ленты вдоль длинной оси большого корпуса под профлистом.
const SKYLIGHT_MIN_AREA: f32 = 700.0;
const SKYLIGHT_WIDTH: f32 = 2.2;
const SKYLIGHT_LENGTH_SHARE: f32 = 0.62;
const SKYLIGHT_HEIGHT: f32 = 0.5;
/// Вторая лента — если поперёк дома есть куда её положить.
const SKYLIGHT_SECOND_WIDTH: f32 = 22.0;

/// Труба на коньке скатной крыши.
const CHIMNEY_SIZE: Vec2 = Vec2::new(0.8, 0.8);
const CHIMNEY_HEIGHT: f32 = 1.4;

/// Зубец крепостной стены: вдоль ребра, поперёк и в высоту, м, шаг по ребру и
/// отступ от наружной грани. Кремлёвский «ласточкин хвост» — зубец в
/// человеческий рост и полтора метра по фронту; развилку на его верху сверху
/// не разглядеть, и рисуется он коробкой.
const MERLON_SIZE: Vec2 = Vec2::new(1.3, 0.7);
const MERLON_HEIGHT: f32 = 1.9;
const MERLON_PITCH: f32 = 2.6;
const MERLON_INSET: f32 = 0.05;
const MERLON_TOP: Color = Color::srgb(0.64, 0.38, 0.30);
const MERLON_WALL: Color = Color::srgb(0.62, 0.34, 0.27);

/// Цвета оборудования: оцинковка и бетон, всё в холодном сером. Фонарь
/// светлее и голубее — это стекло; труба кирпичная.
const EQUIPMENT_TOP: Color = Color::srgb(0.60, 0.60, 0.59);
const EQUIPMENT_WALL: Color = Color::srgb(0.50, 0.50, 0.50);
const PENTHOUSE_TOP: Color = Color::srgb(0.55, 0.54, 0.52);
const PENTHOUSE_WALL: Color = Color::srgb(0.62, 0.61, 0.58);
const SKYLIGHT_TOP: Color = Color::srgb(0.74, 0.77, 0.79);
const SKYLIGHT_WALL: Color = Color::srgb(0.60, 0.62, 0.64);
const CHIMNEY_TOP: Color = Color::srgb(0.38, 0.31, 0.28);
const CHIMNEY_WALL: Color = Color::srgb(0.46, 0.36, 0.31);
/// Насколько тень оборудования темнее самой кровли. Тень непрозрачная —
/// слой зданий рисуется без блендинга, и полупрозрачная тут не смешалась бы.
const CLUTTER_SHADOW_MIX: f32 = 0.30;

/// Коробка на крыше: основание (CCW, уже в координатах нарисованной кровли),
/// настоящая высота над ней, чем красить и докуда пускать тень.
///
/// `reach` считается **в момент раскладки**, а не отрисовки, и это не
/// оптимизация, а развязка: он зависит от контура дома и от сдвига кровли, а
/// у того, кто рисует, на руках только сам предмет. Пока вылет был аргументом
/// [`push_items`], здание и сдвиг ехали туда третьим и четвёртым слагаемым
/// одной тройки с `items`, и несогласованная пара давала не панику, а тихо
/// неверную обрезку тени.
pub(super) struct RoofItem {
    base: [Vec2; 4],
    height: f32,
    top: Color,
    wall: Color,
    reach: f32,
}

/// Оборудование на плоской кровле дома. `lift` — сдвиг нарисованной кровли
/// над контуром: основания коробок приходят уже сдвинутыми, а попадание в
/// контур проверяется до сдвига, по настоящему пятну.
pub(super) fn flat_roof_items(building: &PolyArea, look: &RoofLook, lift: Vec2) -> Vec<RoofItem> {
    // развилки «кому что стоит на кровле» уже разобраны классификатором
    let class = classify(building, None);
    if !roof_clutter_allowed(&class, look) {
        return Vec::new();
    }
    let axis = look.frame.axis;
    let perp = Vec2::new(-axis.y, axis.x);
    let Some(frame) = Frame::of(building, axis, perp) else {
        return Vec::new();
    };

    let mut rng = Lcg::new(look.frame.seed.to_bits() ^ 0x9E37_79B9);
    let mut items = Vec::new();
    let area = frame.footprint;
    let soft = matches!(
        look.kind,
        RoofKind::Bitumen | RoofKind::Gravel | RoofKind::Membrane
    );

    // Решётка зенитных фонарей — **первой из всего**, и это не вкусовщина.
    // Она одна тут кладётся не броском, а по сетке, и подвинуться ей некуда:
    // фонарь стоит по колоннам каркаса. Всё остальное оборудование ищет себе
    // место восемью попытками ([`place`]) и обходит занятое само, так что
    // ставить его первым значило бы дырявить решётку машинным помещением —
    // а на фотографии наоборот, агрегаты стоят в промежутках между фонарями.
    if class.is_big_box() {
        push_skylight_grid(&mut items, &frame, building, lift);
        push_plant(&mut items, &frame, building, &mut rng, lift);
    }

    // машинное помещение — у мягкой кровли крупного дома
    if soft && area >= PENTHOUSE_MIN_AREA {
        let count = if area >= PENTHOUSE_SECOND_AREA { 2 } else { 1 };
        for _ in 0..count {
            place(
                &mut items,
                &frame,
                building,
                &mut rng,
                lift,
                PENTHOUSE_SIZE,
                PENTHOUSE_HEIGHT,
                PENTHOUSE_TOP,
                PENTHOUSE_WALL,
            );
        }
    }

    // ленты зенитных фонарей — большой корпус под профлистом
    if look.kind == RoofKind::Corrugated && area >= SKYLIGHT_MIN_AREA {
        let length = frame.length * SKYLIGHT_LENGTH_SHARE;
        let lanes = if frame.width >= SKYLIGHT_SECOND_WIDTH {
            vec![-0.22, 0.22]
        } else {
            vec![0.0]
        };
        for share in lanes {
            let center = frame.center() + perp * (share * frame.width);
            let size = Vec2::new(length, SKYLIGHT_WIDTH);
            // лента — такой же предмет кровли, как коробка, и место под неё
            // проверяется тем же [`clear`]: сейчас две ленты и так расходятся
            // (полосы в 0.44 ширины дома при ширине от `SKYLIGHT_SECOND_WIDTH`),
            // но правило «оборудование не садится на оборудование» должно
            // держаться постройкой, а не порядком, в котором предметы кладутся
            if let Some(base) = fit(building, center, size, axis, perp, lift)
                && clear(&items, &base, axis, perp)
            {
                items.push(RoofItem {
                    base,
                    height: SKYLIGHT_HEIGHT,
                    top: SKYLIGHT_TOP,
                    wall: SKYLIGHT_WALL,
                    reach: shadow_reach(building, lift, &base),
                });
            }
        }
    }

    // блоки кондиционеров — торговля и казённые здания
    if class.carries_ac() {
        let count = ((area / UNIT_AREA_PER) as usize).clamp(1, UNIT_MAX);
        for _ in 0..count {
            place(
                &mut items,
                &frame,
                building,
                &mut rng,
                lift,
                UNIT_SIZE,
                UNIT_HEIGHT,
                EQUIPMENT_TOP,
                EQUIPMENT_WALL,
            );
        }
    }

    // вентшахты — на любой плоской кровле
    let cap = match class.is_big_box() {
        true => VENT_MAX_BIG_BOX,
        false => VENT_MAX,
    };
    let vents = ((area / VENT_AREA_PER) as usize).clamp(1, cap);
    for _ in 0..vents {
        place(
            &mut items,
            &frame,
            building,
            &mut rng,
            lift,
            VENT_SIZE,
            VENT_HEIGHT,
            EQUIPMENT_TOP,
            EQUIPMENT_WALL,
        );
    }
    items
}

/// Решётка зенитных фонарей по всей кровле гипермаркета: квадрат на каждую
/// ячейку сетки, что попал в контур целиком.
///
/// **Это тот самый случай, когда сетка и есть вещь**, а не расстановщик вещи,
/// — исключение из правила [`super::material`] «клеточная сетка ставит
/// признак, но никогда им не является». Ряд гаражных ворот держится на том же
/// основании: фонари на кровле торгового зала стоят по колоннам каркаса, то
/// есть **правильной сеткой**, и дрожание шага здесь было бы не живостью, а
/// ошибкой — на аэрофото они выстроены по линейке.
///
/// Сетка кладётся от середины рамы, чтобы по обоим краям кровли остался
/// одинаковый отступ, а не полный шаг с одного конца и обрезок с другого.
/// Шаг решётки и число узлов вдоль и поперёк — **подогнанный** под размер
/// кровли, а не константа.
///
/// [`GRID_SKYLIGHT_PITCH`] это цель, как `PANEL_WIDTH` у стены и `BAY` у
/// гаражной ленты: на ТРЦ «Макси» (493 × 298 м) тринадцатиметровый шаг даёт
/// под восемьсот узлов, и упереться им в [`GRID_SKYLIGHT_MAX`] значило бы
/// оборвать решётку на полпути — что и было видно на кадре как «квадратики
/// размещены странно»: первые ряды забирали весь предел, дальняя половина
/// кровли оставалась пустой, и правильной сетки не читалось вовсе. Шаг
/// поэтому **растягивается**, пока вся решётка не уложится в предел: у
/// гипермаркета она остаётся тринадцатиметровой, у гигантского ТЦ становится
/// вдвое реже, но покрывает кровлю целиком — а целая решётка и есть то, по
/// чему торговый зал узнаётся сверху.
///
/// `None` — кровля мельче одного шага: решётки на ней не бывает.
fn grid_pitch(length: f32, width: f32) -> Option<(f32, usize, usize)> {
    let mut pitch = GRID_SKYLIGHT_PITCH;
    for _ in 0..GRID_PITCH_TRIES {
        let along = (length / pitch).floor() as usize;
        let across = (width / pitch).floor() as usize;
        if along == 0 || across == 0 {
            return None;
        }
        if along * across <= GRID_SKYLIGHT_MAX {
            return Some((pitch, along, across));
        }
        // сколько раз решётка не влезла по площади, во столько же раз по
        // стороне надо раздвинуть шаг — отсюда корень
        let excess = (along * across) as f32 / GRID_SKYLIGHT_MAX as f32;
        pitch *= excess.sqrt().max(GRID_PITCH_MIN_STEP);
    }
    None
}

fn push_skylight_grid(items: &mut Vec<RoofItem>, frame: &Frame, building: &PolyArea, lift: Vec2) {
    let size = Vec2::splat(GRID_SKYLIGHT_SIDE);
    let Some((pitch, along, across)) = grid_pitch(frame.length, frame.width) else {
        return;
    };
    let start = |count: usize, extent: f32| (extent - (count - 1) as f32 * pitch) / 2.0;
    let (first_u, first_v) = (start(along, frame.length), start(across, frame.width));
    for row in 0..across {
        for column in 0..along {
            let center = frame.origin
                + frame.axis * (first_u + column as f32 * pitch)
                + frame.perp * (first_v + row as f32 * pitch);
            // у Г-образной коробки часть узлов сетки приходится на двор или на
            // воздух за контуром — такой фонарь просто не ставится, ровно как
            // промахнувшаяся вентшахта
            let Some(base) = fit(building, center, size, frame.axis, frame.perp, lift) else {
                continue;
            };
            // «оборудование не садится на оборудование» — общий инвариант
            // модуля, и держаться он обязан **постройкой, а не порядком
            // вызовов**: сегодня решётка кладётся первой и упереться ей не во
            // что, но переставленный вызов не должен молча начать втыкать
            // фонарь в машинное помещение (ровно это и было в первой версии)
            if !clear(items, &base, frame.axis, frame.perp) {
                continue;
            }
            items.push(RoofItem {
                base,
                height: GRID_SKYLIGHT_HEIGHT,
                top: SKYLIGHT_TOP,
                wall: SKYLIGHT_WALL,
                reach: shadow_reach(building, lift, &base),
            });
        }
    }
}

/// Группа приточных установок: несколько агрегатов в ряд, вплотную друг к
/// другу. Рядом, а не вразброс, — на фотографии они и стоят одним блоком у
/// края зала, потому что подключены к одному коллектору; разбросанные по
/// кровле поодиночке, они бы ничем не отличались от вентшахт.
///
/// Место у группы одно на всю ленту: ищется оно обычным броском
/// ([`place`]-подобно), но проверяется сразу целым габаритом, иначе последний
/// агрегат ряда мог бы повиснуть за контуром.
///
/// «[`place`]-подобно» — это и про **попытки**: бросок засчитан, только когда
/// ряд встал целиком. Габарит спрашивает про контур, а занятость проверяется у
/// каждого агрегата, и решётка фонарей на кровле к этому моменту уже лежит
/// ([`flat_roof_items`] кладёт её первой), так что бросок, чей габарит влез, а
/// слоты упёрлись в фонари, — это промах, а не место: шаг решётки 13 м при
/// стороне фонаря 2.8 м оставляет коридор 10.2 м, а группе нужно 9.6. Пока
/// такой бросок считался местом, на узкой коробке 26 × 70 м ряд выходил пустым
/// на двух раскладках из ста, а из трёх агрегатов на 90 × 110 м оставалось
/// меньше трёх почти на половине.
///
/// Если целого ряда не дала ни одна попытка (на 90 × 110 м это 2 посева из
/// 256), кладётся лучший из найденных: неполный ряд всё же лучше кровли без
/// приточной группы, а место под него уже проверено теми же двумя проверками.
fn push_plant(
    items: &mut Vec<RoofItem>,
    frame: &Frame,
    building: &PolyArea,
    rng: &mut Lcg,
    lift: Vec2,
) {
    let count = ((frame.footprint / PLANT_AREA_PER) as usize).clamp(2, PLANT_MAX);
    let step = PLANT_SIZE.y + PLANT_GAP;
    let run = Vec2::new(PLANT_SIZE.x, count as f32 * step - PLANT_GAP);
    let mut best: Vec<RoofItem> = Vec::new();
    for _ in 0..PLACE_TRIES {
        let center = frame.origin
            + frame.axis * rng.range(0.0, frame.length)
            + frame.perp * rng.range(0.0, frame.width);
        if fit(building, center, run, frame.axis, frame.perp, lift).is_none() {
            continue;
        }
        let first = center - frame.perp * ((run.y - PLANT_SIZE.y) / 2.0);
        let mut placed = Vec::new();
        for slot in 0..count {
            let at = first + frame.perp * (slot as f32 * step);
            let Some(base) = fit(building, at, PLANT_SIZE, frame.axis, frame.perp, lift) else {
                continue;
            };
            if !clear(items, &base, frame.axis, frame.perp)
                || !clear(&placed, &base, frame.axis, frame.perp)
            {
                continue;
            }
            placed.push(RoofItem {
                base,
                height: PLANT_HEIGHT,
                top: EQUIPMENT_TOP,
                wall: EQUIPMENT_WALL,
                reach: shadow_reach(building, lift, &base),
            });
        }
        if placed.len() == count {
            items.append(&mut placed);
            return;
        }
        if placed.len() > best.len() {
            best = placed;
        }
    }
    items.append(&mut best);
}

/// Ставится ли на эту кровлю оборудование. На гаражном ряду нет ни машинного
/// помещения, ни вентшахты — там нечего вентилировать и не топят, и коробка
/// на боксе сразу выдаёт генератор; у храма и крепости
/// ([`BuildingClass::is_landmark`]) стоит своё — главы и зубцы.
fn roof_clutter_allowed(class: &BuildingClass, look: &RoofLook) -> bool {
    !matches!(look.kind, RoofKind::GarageRow | RoofKind::GarageBlock) && !class.is_landmark()
}

/// Зубцы по верху крепостной стены — вдоль каждого ребра контура прясла, с
/// шагом [`MERLON_PITCH`]: с воздуха стена кремля узнаётся по пунктиру зубцов
/// и их коротким теням на боевом ходу. Башне (класс `Fortress { tower: true }`)
/// зубцов не положено — она под шатром.
pub(super) fn merlons(building: &PolyArea, lift: Vec2) -> Vec<RoofItem> {
    if !matches!(
        classify(building, None).kind(),
        BuildingKind::Fortress { tower: false }
    ) {
        return Vec::new();
    }
    let orientation = signed_ring_area(&building.outer).signum();
    let ring = &building.outer;
    let mut items = Vec::new();
    for index in 0..ring.len() {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        let length = a.distance(b);
        let Some(along) = (b - a).try_normalize() else {
            continue;
        };
        // внутрь контура — левый перпендикуляр у CCW-кольца
        let left = Vec2::new(-along.y, along.x);
        let inward = left * orientation;
        let count = (length / MERLON_PITCH).floor() as usize;
        if count == 0 {
            continue;
        }
        // зубцы расставлены от середины ребра, чтобы на обоих концах остался
        // одинаковый зазор до угла
        let start = (length - (count - 1) as f32 * MERLON_PITCH) / 2.0;
        for slot in 0..count {
            let center = a
                + along * (start + slot as f32 * MERLON_PITCH)
                + inward * (MERLON_SIZE.y / 2.0 + MERLON_INSET);
            // основание против часовой при любом обходе кольца: пара
            // `along`/`left` правая
            let base = rect(center, MERLON_SIZE, along, left);
            if !base.iter().all(|corner| point_in_area(*corner, building)) {
                continue;
            }
            let lifted = base.map(|corner| corner + lift);
            items.push(RoofItem {
                base: lifted,
                height: MERLON_HEIGHT,
                top: MERLON_TOP,
                wall: MERLON_WALL,
                reach: shadow_reach(building, lift, &lifted),
            });
        }
    }
    items
}

/// Труба на коньке: `ridge` — оба конца конька уже в нарисованных
/// координатах. `building` и `lift` — те же, по которым строилась крыша: по
/// ним трубе считается [`RoofItem::reach`], как и всякой коробке на плоской
/// кровле.
pub(super) fn ridge_chimney(
    building: &PolyArea,
    look: &RoofLook,
    ridge: (Vec2, Vec2),
    lift: Vec2,
) -> Option<RoofItem> {
    if !roof_clutter_allowed(&classify(building, None), look) {
        return None;
    }
    let along = (ridge.1 - ridge.0).try_normalize()?;
    let length = (ridge.1 - ridge.0).length();
    if length < 2.0 * CHIMNEY_SIZE.x {
        return None;
    }
    let mut rng = Lcg::new(look.frame.seed.to_bits() ^ 0x85EB_CA6B);
    let at = ridge.0 + along * rng.range(0.25, 0.75) * length;
    let across = Vec2::new(-along.y, along.x);
    let base = rect(at, CHIMNEY_SIZE, along, across);
    Some(RoofItem {
        base,
        height: CHIMNEY_HEIGHT,
        top: CHIMNEY_TOP,
        wall: CHIMNEY_WALL,
        reach: shadow_reach(building, lift, &base),
    })
}

/// Коробки в меш: сначала непрозрачная тень каждой, потом сама коробка —
/// видимые стены и верх. В плоских режимах (`lean` не задан) остаётся
/// тень и верх, то есть коробка сверху.
pub(super) fn push_items(
    builder: &mut MeshBuilder,
    items: &[RoofItem],
    lean: Option<Lean>,
    roof: Srgba,
) {
    if items.is_empty() {
        return;
    }
    let shadow_color: LinearRgba = roof.mix(&Srgba::BLACK, CLUTTER_SHADOW_MIX).into();
    // метка оборудования: дальние ступени зума прячут его в шейдере
    // (`RoofParams::clutter`), а не пересборкой слоя
    builder.set_clutter();
    for item in items {
        // тень — свип основания по свету: два ребра силуэта плюс сдвинутое
        // основание. Ровно та же конструкция, что у теней самих домов, и по
        // той же причине: у выпуклого прямоугольника свип двух теневых рёбер
        // и есть недостающая часть объединения, а выпуклую оболочку строить
        // не приходится
        // тень обрезана краем кровли ([`RoofItem::reach`], посчитанным в
        // момент раскладки). Физически она бы через край перевалила — и на
        // снимке переваливает, — но рисуется она цветом этой крыши и
        // непрозрачной, в общем меше зданий: на дефолтных 59° метровая
        // вентшахта укладывается в отступ от края (`EDGE_MARGIN`), а на 15°
        // машинное помещение даёт одиннадцать метров и тёмная полоса уехала бы
        // с крыши на соседний дом и на землю. То есть портится это ровно на том
        // конце ползунка, ради которого высота солнца и стала ручкой
        let offset = shadow::offset(item.height).clamp_length_max(item.reach);
        for (a, b) in silhouette_edges(&item.base, shadow_dir()) {
            builder.push_quad([a, b, b + offset, a + offset], shadow_color);
        }
        builder.push_quad(item.base.map(|point| point + offset), shadow_color);

        let lift = lean.map_or(Vec2::ZERO, |lean| lean.ridge(item.height));
        if let Some(lean) = lean {
            for (a, b) in silhouette_edges(&item.base, -lean.dir()) {
                let (bottom, top) = wall_colors(item.wall.to_srgba(), a, b, lean.dir());
                builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
            }
        }
        builder.push_quad(item.base.map(|point| point + lift), item.top.to_linear());
    }
    // дальше в меш идёт уже не оборудование, а рамка — та же, что ставил сюда
    // прежний `set_roof(None)`
    builder.set_roof(None);
}
