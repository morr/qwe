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
//! иначе субпиксельные коробки мерцают при панораме.

use bevy::color::Mix;
use bevy::prelude::*;

use super::Lean;
use super::layers::{silhouette_edges, wall_colors};
use super::material::{RoofKind, RoofLook};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{is_big_box, is_fortress_tower, point_in_area, signed_ring_area};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};
use crate::map::seed::Lcg;
use crate::map::shadow;
use crate::map::shadow_dir;

/// Сколько мест перебрать, прежде чем отказаться от коробки. Одна попытка
/// на узком корпусе почти всегда промахивалась: машинное помещение 5 × 3.5 м
/// целиком укладывается в двенадцатиметровый дом лишь в узкой полосе. Шесть
/// хватало, пока место проверялось только на попадание в контур; с проверкой
/// на занятость ([`clear`]) десятая вентшахта на плотной кровле отбрасывается
/// куда чаще, и две попытки сверху возвращают её.
const PLACE_TRIES: usize = 8;

/// Зазор между соседними коробками, м. Ноль означал бы «можно вплотную», а
/// два блока кондиционера стенка в стенку читаются как один длинный ящик.
const CLUTTER_GAP: f32 = 0.5;

/// Отступ оборудования от края кровли, м, и доля меньшей стороны дома —
/// на узком корпусе метровый отступ съедает всю крышу.
const EDGE_MARGIN: f32 = 1.6;
const EDGE_MARGIN_SHARE: f32 = 0.18;

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
/// вида, а ради вершин: у ТРЦ «Макси» (52 тыс. м²) без него вышло бы под три
/// сотни коробок в одном меше.
const GRID_SKYLIGHT_SIDE: f32 = 2.8;
const GRID_SKYLIGHT_PITCH: f32 = 13.0;
const GRID_SKYLIGHT_HEIGHT: f32 = 0.6;
/// Предел стоит не ради вида, а ради вершин, и взят он замером: фонарь это
/// шесть четырёхугольников с тенью, то есть около двух десятков вершин, так
/// что три сотни на здание — порядка семи тысяч, доли процента слоя зданий
/// (790 тыс. на Туле), и крупноформатных домов в городе полтора десятка.
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
    // на гараже нет ни машинного помещения, ни вентшахты — там нечего
    // вентилировать, и коробка на боксе сразу выдаёт генератор
    if matches!(look.kind, RoofKind::GarageRow | RoofKind::GarageBlock) || is_landmark(building) {
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
    if is_big_box(building) {
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
    if matches!(
        building.building_use,
        BuildingUse::Commercial | BuildingUse::Retail | BuildingUse::Public
    ) {
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
    let cap = match is_big_box(building) {
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
        items.append(&mut placed);
        return;
    }
}

/// Храм и крепость: вентшахта на боевом ходу или печная труба на вальме храма
/// выдают генератор ровно так же, как коробка на гаражном боксе. Над ними
/// стоит своё — главы и зубцы.
fn is_landmark(building: &PolyArea) -> bool {
    building.kind == AreaKind::Kremlin || matches!(building.building_use, BuildingUse::Church(_))
}

/// Зубцы по верху крепостной стены — вдоль каждого ребра контура прясла, с
/// шагом [`MERLON_PITCH`]: с воздуха стена кремля узнаётся по пунктиру зубцов
/// и их коротким теням на боевом ходу. Башне ([`is_fortress_tower`]) зубцов не
/// положено — она под шатром.
pub(super) fn merlons(building: &PolyArea, lift: Vec2) -> Vec<RoofItem> {
    if building.kind != AreaKind::Kremlin || is_fortress_tower(building) {
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
    // печную трубу на гаражном ряду не ставят — там не топят
    if matches!(look.kind, RoofKind::GarageRow | RoofKind::GarageBlock) || is_landmark(building) {
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
    builder.set_roof(None);
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
}

/// Докуда тень коробки дотянется, не съехав с нарисованной кровли: ближайшее
/// пересечение луча тени с контуром здания — из каждого угла основания, по
/// внешнему кольцу и по каждому двору.
///
/// Меряется по **ненадвинутому** контуру, как и попадание коробки в него
/// (`fit`): сдвиг 2.5D переносит кровлю целиком. У скатной крыши контур чуть
/// уже нарисованного ската (настоящая крыша свисает), так что труба на коньке
/// обрезается с запасом в свою пользу — лучше, чем наоборот.
fn shadow_reach(building: &PolyArea, lift: Vec2, base: &[Vec2; 4]) -> f32 {
    let dir = shadow_dir();
    // лучи идут из всех четырёх углов base, сдвинутых на -lift — ребро можно
    // пропустить, только если оно позади каждого из этих истинных начал луча
    let min_origin = base
        .iter()
        .map(|c| (*c - lift).dot(dir))
        .fold(f32::MAX, f32::min);
    let mut reach = f32::MAX;
    for ring in std::iter::once(&building.outer).chain(building.holes.iter()) {
        for (index, &a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            // ребро целиком позади всех лучей — ни один их не догонит
            if a.dot(dir) < min_origin && b.dot(dir) < min_origin {
                continue;
            }
            for corner in base {
                if let Some(hit) = ray_hits_segment(*corner - lift, dir, a, b) {
                    reach = reach.min(hit);
                }
            }
        }
    }
    reach
}

/// Длина по лучу до пересечения с отрезком, если луч его пересекает.
fn ray_hits_segment(from: Vec2, dir: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let edge = b - a;
    let denom = dir.perp_dot(edge);
    if denom.abs() < 1e-6 {
        return None;
    }
    let to_a = a - from;
    let along_ray = to_a.perp_dot(edge) / denom;
    let along_edge = to_a.perp_dot(dir) / denom;
    (along_ray >= 0.0 && (0.0..=1.0).contains(&along_edge)).then_some(along_ray)
}

/// Рама дома: длинная ось, её длина и ширина, начало — угол описанного по
/// этой оси прямоугольника, уже с отступом от края.
struct Frame {
    origin: Vec2,
    axis: Vec2,
    perp: Vec2,
    length: f32,
    width: f32,
    footprint: f32,
}

impl Frame {
    /// Рама с отступом от края; `None` — после отступа не осталось места.
    fn of(building: &PolyArea, axis: Vec2, perp: Vec2) -> Option<Self> {
        let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
        let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
        for point in &building.outer {
            min_u = min_u.min(point.dot(axis));
            max_u = max_u.max(point.dot(axis));
            min_v = min_v.min(point.dot(perp));
            max_v = max_v.max(point.dot(perp));
        }
        let (length, width) = (max_u - min_u, max_v - min_v);
        let margin = EDGE_MARGIN.min(width.min(length) * EDGE_MARGIN_SHARE);
        let (length, width) = (length - 2.0 * margin, width - 2.0 * margin);
        if length <= 0.0 || width <= 0.0 {
            return None;
        }
        Some(Self {
            origin: axis * (min_u + margin) + perp * (min_v + margin),
            axis,
            perp,
            length,
            width,
            footprint: signed_ring_area(&building.outer).abs(),
        })
    }

    fn center(&self) -> Vec2 {
        self.origin + self.axis * (self.length / 2.0) + self.perp * (self.width / 2.0)
    }
}

/// Одна попытка поставить коробку: точка в раме по ГПСЧ, проверка на попадание
/// в контур и на свободное место. Промах просто теряется — дырявый ряд вентшахт
/// на Г-образном доме выглядит естественнее, чем шахта, висящая над двором.
#[allow(clippy::too_many_arguments)]
fn place(
    items: &mut Vec<RoofItem>,
    frame: &Frame,
    building: &PolyArea,
    rng: &mut Lcg,
    lift: Vec2,
    size: Vec2,
    height: f32,
    top: Color,
    wall: Color,
) {
    for _ in 0..PLACE_TRIES {
        let center = frame.origin
            + frame.axis * rng.range(0.0, frame.length)
            + frame.perp * rng.range(0.0, frame.width);
        let Some(base) = fit(building, center, size, frame.axis, frame.perp, lift) else {
            continue;
        };
        if !clear(items, &base, frame.axis, frame.perp) {
            continue;
        }
        items.push(RoofItem {
            base,
            height,
            top,
            wall,
            reach: shadow_reach(building, lift, &base),
        });
        return;
    }
}

/// Свободно ли место под коробку. Все предметы плоской кровли разложены в
/// одной раме (`axis`/`perp`) — и вентшахты, и блоки кондиционеров, и лента
/// зенитного фонаря, — так что пересечение двух оснований это обычная проверка
/// двух отрезков по каждой из двух осей, с зазором [`CLUTTER_GAP`] между ними.
///
/// Без неё вентшахта садилась на машинное помещение, а блок кондиционера — на
/// вентшахту: [`fit`] спрашивает только про контур дома и ничего не знает о
/// том, что на кровле уже стоит.
fn clear(items: &[RoofItem], base: &[Vec2; 4], axis: Vec2, perp: Vec2) -> bool {
    let span = |corners: &[Vec2; 4], dir: Vec2| {
        corners
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), corner| {
                let at = corner.dot(dir);
                (lo.min(at), hi.max(at))
            })
    };
    let apart = |a: (f32, f32), b: (f32, f32)| a.1 + CLUTTER_GAP <= b.0 || b.1 + CLUTTER_GAP <= a.0;
    let (along, across) = (span(base, axis), span(base, perp));
    !items
        .iter()
        .any(|item| !apart(along, span(&item.base, axis)) && !apart(across, span(&item.base, perp)))
}

/// Основание коробки, если все четыре угла легли внутрь контура. Проверка по
/// **ненадвинутому** пятну: сдвиг 2.5D переносит кровлю целиком, и то, что
/// внутри контура, останется внутри нарисованной крыши.
fn fit(
    building: &PolyArea,
    center: Vec2,
    size: Vec2,
    axis: Vec2,
    perp: Vec2,
    lift: Vec2,
) -> Option<[Vec2; 4]> {
    let base = rect(center, size, axis, perp);
    base.iter()
        .all(|corner| point_in_area(*corner, building))
        .then(|| base.map(|corner| corner + lift))
}

/// Прямоугольник `size` вокруг точки, повёрнутый в раму (CCW при
/// правой паре `axis`/`perp`).
fn rect(center: Vec2, size: Vec2, axis: Vec2, perp: Vec2) -> [Vec2; 4] {
    let half_axis = axis * (size.x / 2.0);
    let half_perp = perp * (size.y / 2.0);
    [
        center - half_axis - half_perp,
        center + half_axis - half_perp,
        center + half_axis + half_perp,
        center - half_axis + half_perp,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::AreaKind;
    use crate::map::sun::SUN_ELEVATION_MIN;

    fn building(outer: Vec<Vec2>, building_use: BuildingUse) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use,
            height: Some(15.0),
            entrances: Vec::new(),
            colours: Default::default(),
        }
    }

    fn block(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    /// Луч тени останавливается на краю кровли: азимут 270° кладёт тень строго
    /// на восток, и от коробки в `x = 4..6` до стены `x = 20` остаётся 14 м —
    /// по ближайшему из углов, а не по дальнему.
    #[test]
    fn the_shadow_reach_stops_at_the_roof_edge() {
        let _sun = crate::map::sun_at(270.0, 45.0);
        assert!(
            (shadow_dir() - Vec2::X).length() < 1e-5,
            "{:?}",
            shadow_dir()
        );
        let slab = building(block(20.0, 20.0), BuildingUse::Other);
        let base = rect(Vec2::new(5.0, 10.0), Vec2::splat(2.0), Vec2::X, Vec2::Y);
        let reach = shadow_reach(&slab, Vec2::ZERO, &base);
        assert!((reach - 14.0).abs() < 1e-3, "{reach}");
    }

    /// На низком солнце тень оборудования обязана остаться на крыше: рисуется
    /// она непрозрачной и цветом этой кровли, так что съехавшая полоса легла бы
    /// тёмной чертой на соседний дом и на землю.
    #[test]
    fn a_low_sun_keeps_the_equipment_shadow_on_the_roof() {
        let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let mut items = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert!(!items.is_empty());
        // и отдельно — машинное помещение у самого края с наветренной стороны:
        // разложенное оборудование до края может и не достать, а зажим нужен
        // именно ему
        let base = rect(Vec2::new(50.0, 8.0), PENTHOUSE_SIZE, Vec2::X, Vec2::Y);
        items.push(RoofItem {
            base,
            height: PENTHOUSE_HEIGHT,
            top: PENTHOUSE_TOP,
            wall: PENTHOUSE_WALL,
            reach: shadow_reach(&slab, Vec2::ZERO, &base),
        });
        let mut clamped = 0;
        for item in &items {
            let wanted = shadow::length(item.height);
            // тот самый вылет, который увидит `push_items`, — из предмета, а
            // не пересчитанный тестом заново
            let reach = item.reach;
            if reach < wanted {
                clamped += 1;
            }
            let offset = shadow_dir() * wanted.min(reach);
            for corner in item.base {
                // зажатая тень кончается ровно на стене, и точка на самом
                // контуре в `point_in_area` уже наружу — отступаем на промилле
                let far = corner + offset * 0.999;
                assert!(point_in_area(far, &slab), "{far:?} is off the roof");
            }
        }
        // без зажима хоть одна тень с этой крыши уходит: машинное помещение в
        // три метра даёт на 15° одиннадцать метров при шестнадцатиметровой
        // ширине корпуса
        assert!(clamped > 0, "nothing was clamped, the case is not covered");
    }

    /// Гипермаркет от магазина у дома отличается кровлей, и это разница по
    /// **размеру**, а не по тегу: решётка зенитных фонарей и группа приточных
    /// установок появляются только на крупноформатном пятне, а один и тот же
    /// `shop=supermarket` на 300 м² остаётся обычной мелкой коробкой.
    #[test]
    fn a_hypermarket_roof_is_a_grid_of_skylights_and_a_corner_shop_is_not() {
        let _sun = crate::map::default_sun();
        let hyper = building(block(90.0, 110.0), BuildingUse::Retail);
        assert!(is_big_box(&hyper));
        let look = super::super::material::roof_look(&hyper);
        let items = flat_roof_items(&hyper, &look, Vec2::ZERO);
        let skylights = items.iter().filter(|item| item.top == SKYLIGHT_TOP).count();
        // 110 × 90 м с отступом от края — не меньше шести шагов по 13 м вдоль
        // и не меньше пяти поперёк
        assert!(skylights >= 30, "{skylights} skylights on a hypermarket");
        assert!(skylights <= GRID_SKYLIGHT_MAX);
        // группа приточных установок стоит одним рядом
        let plant = items
            .iter()
            .filter(|item| (item.height - PLANT_HEIGHT).abs() < 1e-3)
            .count();
        assert!(plant >= 2, "{plant} plant units");

        // тот же класс на мелком пятне: ни одного фонаря и ни одной установки
        let shop = building(block(14.0, 20.0), BuildingUse::Retail);
        assert!(!is_big_box(&shop));
        let look = super::super::material::roof_look(&shop);
        let items = flat_roof_items(&shop, &look, Vec2::ZERO);
        assert!(items.iter().all(|item| item.top != SKYLIGHT_TOP));
        assert!(
            items
                .iter()
                .all(|item| (item.height - PLANT_HEIGHT).abs() > 1e-3)
        );
    }

    /// Фонари решётки стоят **по линейке**: шаг между соседями в ряду —
    /// ровно [`GRID_SKYLIGHT_PITCH`]. Это тот случай, когда сетка и есть
    /// вещь, и дрожание шага тут было бы ошибкой, а не живостью.
    #[test]
    fn the_skylight_grid_keeps_its_pitch() {
        let _sun = crate::map::default_sun();
        let hyper = building(block(90.0, 110.0), BuildingUse::Retail);
        let look = super::super::material::roof_look(&hyper);
        let items = flat_roof_items(&hyper, &look, Vec2::ZERO);
        let mut centres: Vec<Vec2> = items
            .iter()
            .filter(|item| item.top == SKYLIGHT_TOP)
            .map(|item| item.base.iter().sum::<Vec2>() / 4.0)
            .collect();
        assert!(centres.len() > 2);
        // контур осепараллелен, так что рама совпадает с мировыми осями:
        // сортируем по строке, потом по столбцу
        centres.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
        let mut checked = 0;
        for pair in centres.windows(2) {
            if (pair[1].y - pair[0].y).abs() > 1e-3 {
                continue;
            }
            assert!(
                (pair[1].x - pair[0].x - GRID_SKYLIGHT_PITCH).abs() < 1e-3,
                "{:?} → {:?}",
                pair[0],
                pair[1]
            );
            checked += 1;
        }
        assert!(checked > 0, "no two skylights shared a row");
    }

    /// На гиганте решётка **разрежается, а не обрывается**: шаг подгоняется
    /// так, чтобы вся она уложилась в предел и накрыла кровлю целиком. Пока
    /// шаг был константой, ТРЦ «Макси» (493 × 298 м) забирал предел первыми
    /// рядами, и дальняя половина кровли оставалась пустой — сообщено по
    /// кадру как «квадратики размещены странно».
    #[test]
    fn a_giant_roof_thins_the_grid_instead_of_cutting_it_off() {
        let _sun = crate::map::default_sun();
        let maxi = building(block(298.0, 493.0), BuildingUse::Retail);
        let look = super::super::material::roof_look(&maxi);
        let items = flat_roof_items(&maxi, &look, Vec2::ZERO);
        let centres: Vec<Vec2> = items
            .iter()
            .filter(|item| item.top == SKYLIGHT_TOP)
            .map(|item| item.base.iter().sum::<Vec2>() / 4.0)
            .collect();
        assert!(centres.len() > GRID_SKYLIGHT_MAX / 2, "{}", centres.len());
        assert!(centres.len() <= GRID_SKYLIGHT_MAX);
        // решётка дотягивается до обоих концов: контур осепараллелен, так что
        // крайние узлы обязаны стоять у обоих краёв рамы, а не в одной трети
        let (lo, hi) = centres.iter().fold((f32::MAX, f32::MIN), |(lo, hi), at| {
            (lo.min(at.x), hi.max(at.x))
        });
        assert!(lo < 60.0 && hi > 433.0, "grid spans {lo}..{hi} of 0..493");
        // и шаг остался единым — она реже, но по-прежнему решётка
        let mut rows: Vec<f32> = centres.iter().map(|at| at.x).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        for pair in rows.windows(2) {
            let step = pair[1] - pair[0];
            assert!(
                step >= GRID_SKYLIGHT_PITCH - 1e-3,
                "{step} m between columns"
            );
        }
    }

    #[test]
    fn a_block_gets_equipment_and_a_shed_does_not() {
        let _sun = crate::map::default_sun();
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let items = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert!(!items.is_empty(), "a 960 m2 block carries equipment");

        // сарай 3 × 3: после отступа от края ставить некуда
        let shed = building(block(3.0, 3.0), BuildingUse::Other);
        let look = super::super::material::roof_look(&shed);
        assert!(flat_roof_items(&shed, &look, Vec2::ZERO).len() <= 1);
    }

    #[test]
    fn equipment_stays_inside_the_footprint() {
        let _sun = crate::map::default_sun();
        // Г-образный контур: рама прямоугольна, дом — нет
        let ell = building(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(60.0, 0.0),
                Vec2::new(60.0, 14.0),
                Vec2::new(20.0, 14.0),
                Vec2::new(20.0, 50.0),
                Vec2::new(0.0, 50.0),
            ],
            BuildingUse::Apartments,
        );
        let look = super::super::material::roof_look(&ell);
        for item in flat_roof_items(&ell, &look, Vec2::ZERO) {
            for corner in item.base {
                assert!(point_in_area(corner, &ell), "{corner:?} is off the roof");
            }
        }
    }

    /// Две коробки не садятся одна на другую. Места берутся из ГПСЧ, и пока
    /// проверялось только попадание в контур, блок кондиционера вставал на
    /// вентшахту — на детском саду 9 × 17 м (Тула, way 234273437), где кровля
    /// несёт ровно эти две коробки и промахнуться мимо соседа почти не в чем.
    #[test]
    fn equipment_never_sits_on_equipment() {
        let _sun = crate::map::default_sun();
        // посев берётся от первой вершины контура, так что одна кровля — это
        // одна раскладка из многих: дом переставляется по карте, и правило
        // проверяется на сотне посевов, а не на том единственном, который
        // случайно лёг удачно
        for (width, length, building_use) in [
            (9.0, 17.0, BuildingUse::Public),
            (16.0, 60.0, BuildingUse::Apartments),
            (40.0, 60.0, BuildingUse::Commercial),
            (30.0, 90.0, BuildingUse::Industrial),
            // гипермаркет — единственная кровля, где предмет кладётся **не
            // броском, а по сетке**, и первая её версия ставила фонарь прямо
            // на уже стоящее машинное помещение (сообщено по кадру). Правило
            // общее, и проверяться оно обязано и на этой раскладке тоже
            (90.0, 110.0, BuildingUse::Retail),
            // узкая коробка: сетка почти упирается в отступ от края, а
            // агрегатам приточной группы остаётся одна полоса
            (26.0, 70.0, BuildingUse::Retail),
        ] {
            let mut laid = 0;
            for step in 0..100 {
                let at = Vec2::new(step as f32 * 13.0, step as f32 * 7.0);
                let outer = block(width, length)
                    .into_iter()
                    .map(|corner| corner + at)
                    .collect();
                let house = building(outer, building_use);
                let look = super::super::material::roof_look(&house);
                let items = flat_roof_items(&house, &look, Vec2::ZERO);
                laid += items.len();
                for (index, item) in items.iter().enumerate() {
                    for other in &items[index + 1..] {
                        assert!(
                            !boxes_overlap(&item.base, &other.base),
                            "{width}x{length} {building_use:?} at {at:?}: \
                             {:?} sits on {:?}",
                            item.base,
                            other.base
                        );
                    }
                }
            }
            assert!(laid > 100, "{width}x{length} {building_use:?} is empty");
        }
    }

    /// Пересекаются ли два выпуклых четырёхугольника — по разделяющей оси, на
    /// нормалях рёбер обоих. Тест намеренно не знает про общую раму, в которой
    /// раскладывается оборудование, и поймал бы промах и в повёрнутой паре.
    fn boxes_overlap(a: &[Vec2; 4], b: &[Vec2; 4]) -> bool {
        let span = |quad: &[Vec2; 4], dir: Vec2| {
            quad.iter().fold((f32::MAX, f32::MIN), |(lo, hi), corner| {
                let at = corner.dot(dir);
                (lo.min(at), hi.max(at))
            })
        };
        for quad in [a, b] {
            for index in 0..quad.len() {
                let edge = quad[(index + 1) % quad.len()] - quad[index];
                let Some(normal) = Vec2::new(-edge.y, edge.x).try_normalize() else {
                    continue;
                };
                let (a_lo, a_hi) = span(a, normal);
                let (b_lo, b_hi) = span(b, normal);
                if a_hi <= b_lo || b_hi <= a_lo {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn the_placement_is_stable() {
        let _sun = crate::map::default_sun();
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let first = flat_roof_items(&slab, &look, Vec2::ZERO);
        let second = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.base, b.base);
        }
    }

    /// Тень трубы на коньке обязана остаться на крыше. Наклон конька сдвигает
    /// базовую точку вверх; передача сдвига в shadow_reach обязана быть полной
    /// (lift + ridge_offset).
    #[test]
    fn ridge_chimney_shadow_stays_on_roof_with_low_sun() {
        let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
        let house = building(block(8.0, 10.0), BuildingUse::Other);
        let look = super::super::material::roof_look(&house);
        // конёк двускатной крыши проходит по средней линии короткой стороны
        let ridge = (Vec2::new(4.0, 0.0), Vec2::new(4.0, 10.0));
        // ridge_offset на типовой крыше — несколько дециметров; тест ищет баг,
        // когда сдвиг конька не передан, поэтому lift=Vec2::ZERO даст краткие
        // тени. С правильным ridge_offset эта область покрывается.
        let lift = Vec2::new(0.5, 0.5);
        if let Some(item) = ridge_chimney(&house, &look, ridge, lift) {
            let offset = shadow::offset(item.height).clamp_length_max(item.reach);
            for corner in item.base {
                let far = corner + offset * 0.999;
                assert!(
                    point_in_area(far, &house),
                    "{far:?} is off the house boundary"
                );
            }
        }
    }
}
