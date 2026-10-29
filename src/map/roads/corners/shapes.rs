use std::f32::consts::PI;

use bevy::prelude::*;

use crate::map::along::{arclengths, nearest_on_path};
use crate::map::meshing::{arc_steps, miter_offsets};
use crate::map::osm::RoadClass;
use crate::map::osm::model::ring_area;

use super::{
    Arm, BEND_FIT, BEND_OVERLAP, BEND_REACH, BEND_SETTLE, KERB_PAD_RUN, KERB_PAD_WIDTH, MAX_ANGLE,
    MIN_ANGLE, MIN_OUTER, MIN_RADIUS, NOSE_MAX_ANGLE, OBTUSE_MAX, OUTER_OVERLAP, OVERLAP,
    ccw_angle, ccw_between,
};

/// Наружный угол от луча `first` против часовой стрелки до луча `second`, если
/// просвет между ними шире развёрнутого: веер от узла с радиусом от одной
/// полуширины к другой — то, что раньше давали круглые торцы лент. Первая
/// вершина — центр веера, чуть позади узла, стороны заходят под торцы лент
/// на [`OUTER_OVERLAP`]. Стороны — по **торцевым кромкам лент**
/// ([`Arm::butt`]): прямой торец идёт поперёк исходного крайнего звена оси,
/// а направление луча смотрит на вершину ленты после слияния, — на стыке
/// двух way одной улицы с изломом те расходятся, и веер по лучу оставлял
/// между собой и торцами клиновидную щель поперёк асфальта (Тула, Фрунзе у
/// съезда, N1). Просвет меряется по-прежнему по лучам узловой оси: порог
/// [`MIN_OUTER`] стоит на шуме округления оси, а не торцов.
pub(super) fn outer_corner(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= PI + MIN_OUTER {
        return None;
    }
    let sweep = angle - PI;
    let from_dir = first.butt.unwrap_or(first.direction);
    let to_dir = second.butt.unwrap_or(second.direction);
    let from = from_dir.perp();
    let steps = arc_steps(halves.0.max(halves.1), sweep).max(1);
    let middle = Vec2::from_angle(sweep / 2.0).rotate(from);
    let mut outline = Vec::with_capacity(steps + 2);
    outline.push(node - middle * OUTER_OVERLAP);
    outline.push(node + from * halves.0 + from_dir * OUTER_OVERLAP);
    for step in 1..steps {
        let share = step as f32 / steps as f32;
        let radius = halves.0 + (halves.1 - halves.0) * share;
        outline.push(node + Vec2::from_angle(sweep * share).rotate(from) * radius);
    }
    outline.push(node - to_dir.perp() * halves.1 + to_dir * OUTER_OVERLAP);
    Some(outline)
}

/// Во сколько раз дуга скругления режется мельче общего шага
/// ([`arc_steps`]) у всех угловых кусков: стыки кусков общего шага
/// (12–17° на угол) читались гранями «ломаной» кромки тротуара и газона
/// (Тула, Фёдора Смирнова × Красноармейский, 3399 3835). Втрое — излом
/// 4–6°. Это верно и для дорожной группы: у обычного перекрёстка её дуга —
/// сама линия бордюра (ленты заканчиваются у узла прямыми торцами), и её
/// стыки видны так же.
const KERB_ARC_SUBDIVISION: usize = 3;

/// Скругление угла от луча `first` против часовой стрелки до луча `second`:
/// `halves` — полуширины полос, по краям которых сходится угол, `radius` —
/// радиус дуги в этом углу. Дуга режется в [`KERB_ARC_SUBDIVISION`] раз
/// мельче общего шага.
pub(super) fn fillet(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    radius: f32,
) -> Option<Vec<Vec2>> {
    let mut arc = fillet_arc(node, first, second, halves, radius)?;
    arc.steps *= KERB_ARC_SUBDIVISION;
    // прямые стороны заходят под ленты на `OVERLAP`: сторона, совпадающая с
    // краем ленты, но не делящая с ней вершин, растеризуется с пропусками —
    // по краю проезда шла пунктирная щель со светлым тротуаром под ней
    let mut outline = Vec::with_capacity(arc.steps + 4);
    outline.push(arc.corner - (arc.side_first + arc.side_second) * OVERLAP);
    outline.push(arc.on_first - arc.side_first * OVERLAP);
    outline.extend(arc.points(0.0));
    outline.push(arc.on_second - arc.side_second * OVERLAP);
    Some(outline)
}

/// **Изогнутое скругление** (bend) угла от луча `first` против часовой стрелки
/// до луча `second` — там, где прямое ([`fillet_arc`]) не легло, потому что
/// кромка луча прямая меньше касательной: дуга кольца (кольцо Ø 29 м идёт
/// хордами по два метра, и прямой пробег его луча — одна хорда), подход,
/// загнутый к кольцу (`rings::bend_approach`). Без него угол въезда в
/// кольцо оставался ступенькой, и из-под неё торчал торец полосы тротуара
/// клином (Белгород, Чапаева у кольца, R26).
///
/// Кромки — **осевые лучей** ([`Arm::trail`]), сдвинутые на полуширины
/// `halves`, как у носа; угол — их первое пересечение от узла, дуга радиуса
/// `radius` садится на обе кромки: центр сдвигается [`BEND_SETTLE`] раз, пока
/// не встанет на радиус от каждой. Касание не нашлось в [`BEND_REACH`] — `None`.
/// Контур — от угла по кромке первого до касания, дуга, по кромке второго
/// назад; стороны заходят под ленты на [`OVERLAP`]. Он не выпукл (кромка
/// кольца выгнута в угол) — триангулируется целиком ([`KerbReturns::bends`]).
/// Клин под лучом ([`Arm::slope`]) — не сюда: у него кромка своя. Угол
/// острее [`NOSE_MAX_ANGLE`] — тоже: там нос, а у кольца подход, влитый под
/// 25° (`rings::ENTRY_ANGLE`), — это клин островка (`roads/gores.rs`), и дуга
/// в шесть метров заливала бы его асфальтом на двадцать метров вглубь.
pub(super) fn bend(
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    radius: f32,
) -> Option<Vec<Vec2>> {
    if radius < MIN_RADIUS || first.slope[0] != 0.0 || second.slope[1] != 0.0 {
        return None;
    }
    if !(NOSE_MAX_ANGLE..=MAX_ANGLE).contains(&ccw_angle(first, second)) {
        return None;
    }
    // кромка и она же под лентой на `BEND_OVERLAP` — вершина в вершину
    let edges = |arm: &Arm, half: f32| -> [Vec<Vec2>; 2] {
        let trail = within(&arm.trail(), BEND_REACH);
        [half, half - BEND_OVERLAP * half.signum()].map(|half| {
            trail
                .iter()
                .zip(miter_offsets(&trail, false, half))
                .map(|(point, offset)| *point + offset)
                .collect()
        })
    };
    // слева по ходу первого, справа по ходу второго
    let [first_edge, first_under] = edges(first, halves.0);
    let [second_edge, second_under] = edges(second, -halves.1);
    let (corner, [from_first, from_second]) = first_crossing(&first_edge, &second_edge)?;
    // кромки от угла прочь от узла
    let chain = |edge: &[Vec2], from: usize| -> Vec<Vec2> {
        std::iter::once(corner)
            .chain(edge[from + 1..].iter().copied())
            .collect()
    };
    let (first_chain, second_chain) = (
        chain(&first_edge, from_first),
        chain(&second_edge, from_second),
    );
    let direction = |chain: &[Vec2]| (chain[1] - chain[0]).try_normalize();
    let (along_first, along_second) = (direction(&first_chain)?, direction(&second_chain)?);
    let half_angle = along_first.angle_to(along_second).abs() / 2.0;
    if half_angle < MIN_ANGLE / 2.0 {
        return None;
    }
    // касание — на кромках за углом и до конца поиска, дуга — в радиус
    let touches = |centre: Vec2, foot: Vec2, cut: f32, total: f32, radius: f32| {
        cut > OVERLAP && cut < total - OVERLAP && (centre.distance(foot) - radius).abs() < BEND_FIT
    };
    // Класс-радиус может не лечь в карман: за углом кромка скоро кончается, и
    // касательная садится за её конец (Путейская × Демонстрации, Тула: прямой
    // край плеча кончается дугой сквозного отрезка за 3.2 м от узла, и до
    // конца кромки за углом краёв — 7 м, касательной шестиметровой дуги — 7.6).
    // Радиус сжимается, пока обе касательные не сядут на кромки, — как в
    // [`fillet_arc`] сжимается касательная под прямым краем.
    let bisector = (along_first + along_second).try_normalize()?;
    let (_, first_total) = arclengths(&first_chain);
    let (_, second_total) = arclengths(&second_chain);
    let mut radius = radius;
    let (centre, first_foot, first_cut, second_foot, second_cut) = loop {
        let mut centre = corner + bisector * (radius / half_angle.sin());
        for _ in 0..BEND_SETTLE {
            let (first_foot, _) = nearest_on_path(&first_chain, centre)?;
            let (second_foot, _) = nearest_on_path(&second_chain, centre)?;
            let (away_first, away_second) = (centre - first_foot, centre - second_foot);
            centre += away_first.normalize_or_zero() * (radius - away_first.length())
                + away_second.normalize_or_zero() * (radius - away_second.length());
        }
        let (first_foot, first_cut) = nearest_on_path(&first_chain, centre)?;
        let (second_foot, second_cut) = nearest_on_path(&second_chain, centre)?;
        let inside = |cut: f32, total: f32| cut > OVERLAP && cut < total - OVERLAP;
        if (inside(first_cut, first_total) && inside(second_cut, second_total))
            || radius <= MIN_RADIUS
        {
            break (centre, first_foot, first_cut, second_foot, second_cut);
        }
        radius *= 0.8;
    };
    if !touches(centre, first_foot, first_cut, first_total, radius)
        || !touches(centre, second_foot, second_cut, second_total, radius)
    {
        return None;
    }
    // стороны — по кромкам под лентами: вершины кромки до касания
    let under = |edge: &[Vec2], under: &[Vec2], from: usize, cut: f32| -> Vec<Vec2> {
        let mut along = corner.distance(edge[from + 1]);
        let mut points = Vec::new();
        for index in from + 1..edge.len() {
            if index > from + 1 {
                along += edge[index].distance(edge[index - 1]);
            }
            if along >= cut {
                break;
            }
            points.push(under[index]);
        }
        points
    };
    let inward = |foot: Vec2| (foot - centre).normalize_or_zero() * BEND_OVERLAP;
    let mut outline =
        vec![corner + (along_first + along_second).normalize_or_zero() * -BEND_OVERLAP];
    outline.extend(under(&first_edge, &first_under, from_first, first_cut));
    outline.push(first_foot + inward(first_foot));
    let (from, to) = (first_foot - centre, second_foot - centre);
    let sweep = from.angle_to(to);
    // тот же мелкий шаг, что у прямого скругления: кромка гнутого куска —
    // бордюр или кромка дорожки, стыки общего шага читаются гранями
    let steps = arc_steps(radius, sweep.abs()).max(1) * KERB_ARC_SUBDIVISION;
    outline
        .extend((0..=steps).map(|step| {
            centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from)
        }));
    outline.push(second_foot + inward(second_foot));
    let mut back = under(&second_edge, &second_under, from_second, second_cut);
    back.reverse();
    outline.extend(back);
    (outline.len() >= 4 && ring_area(&outline) > 0.01).then_some(outline)
}

/// **Тупой угол со сменой ширины** от луча `first` против часовой стрелки до
/// луча `second`, [`MAX_ANGLE`]..[`OBTUSE_MAX`] (R13): почти продолжение
/// дороги, но в узле перекрёстка, где торцы прямые, а ширины разные. Кромка
/// широкого плеча, продлённая за узел, сходится с кромкой узкого только за
/// узлом по узкому — скругление ([`fillet_arc`]) там не ляжет (угол краёв
/// позади торца широкого), и между прямым торцом широкого и кромкой узкого
/// оставался клин земли, асфальт обрывался срезом (Тула, Одоевский
/// путепровод × Демонстрации, 162°).
///
/// Контур — клин от торца широкого по его продлённой кромке до пологой дуги,
/// касательной к обеим кромкам: касание на широком — не дальше его торца,
/// на узком — не дальше его прямого края ([`Arm::run`]). Первая вершина —
/// в узле чуть под лентой широкого, стороны заходят под ленты на
/// [`OVERLAP`]; веером из первой вершины. Сквозная дорога (оба луча — одна
/// ось), клин под лучом и равные ширины (кромки сходятся перед узлом по
/// обоим лучам) — `None`.
pub(super) fn obtuse_corner(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= MAX_ANGLE || angle >= OBTUSE_MAX {
        return None;
    }
    if std::ptr::eq(first.path, second.path) || first.slope[0] != 0.0 || second.slope[1] != 0.0 {
        return None;
    }
    // только кромка проезжей части и не у кольца: у дорожек ширины разные по
    // классу покрытия, а две дуги кольца — одна замкнутая лента со своим
    // тротуаром (`roads::push_ring_edges`), и клин по тротуарам дуг разной
    // ширины ложился плиткой поперёк газона острова (Тула, кадр R15)
    if first.class != RoadClass::Street
        || second.class != RoadClass::Street
        || first.ring
        || second.ring
    {
        return None;
    }
    // кромки: слева у первого, справа у второго — `node + n·h + u·x`
    let (u1, u2) = (first.direction, second.direction);
    let (n1, n2) = (u1.perp(), -u2.perp());
    let rhs = n2 * halves.1 - n1 * halves.0;
    let determinant = -u1.perp_dot(u2);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-u2) / determinant;
    let s = u1.perp_dot(rhs) / determinant;
    // угол краёв — позади торца широкого и на прямом крае узкого
    let (wide, narrow, behind, on) = match (t >= 0.0, s >= 0.0) {
        (true, false) => (second, first, -s, t),
        (false, true) => (first, second, -t, s),
        _ => return None,
    };
    wide.end?;
    let corner = node + n1 * halves.0 + u1 * t;
    let tangent = behind.min(narrow.run - on);
    if tangent <= OVERLAP {
        return None;
    }
    let half_angle = angle / 2.0;
    let radius = tangent * half_angle.tan();
    let (on_first, on_second) = (corner + u1 * tangent, corner + u2 * tangent);
    let centre = corner + (u1 + u2).normalize() * (radius / half_angle.sin());
    let (from, to) = (on_first - centre, on_second - centre);
    let sweep = from.angle_to(to);
    let steps = arc_steps(radius, sweep.abs()).max(1);
    let wide_first = std::ptr::eq(wide, first);
    let ((wide_normal, wide_half), (narrow_normal, narrow_half)) = if wide_first {
        ((n1, halves.0), (n2, halves.1))
    } else {
        ((n2, halves.1), (n1, halves.0))
    };
    // торец широкого — под его лентой, кромка узкого — под своей
    let wide_end = node + wide_normal * (wide_half - OVERLAP) + wide.direction * OVERLAP;
    let narrow_end = node + narrow_normal * (narrow_half - OVERLAP);
    let arc = (0..=steps)
        .map(|step| centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from));
    let mut outline = Vec::with_capacity(steps + 5);
    outline.push(node + wide.direction * OUTER_OVERLAP);
    if wide_first {
        outline.push(wide_end);
        outline.extend(arc);
        outline.push(on_second - n2 * OVERLAP);
        outline.push(narrow_end);
    } else {
        outline.push(narrow_end);
        outline.push(on_first - n1 * OVERLAP);
        outline.extend(arc);
        outline.push(wide_end);
    }
    (ring_area(&outline).abs() > 0.01).then_some(outline)
}

/// Начало ломаной `path` длиной до `reach` м (последнее звено — целиком).
fn within(path: &[Vec2], reach: f32) -> Vec<Vec2> {
    let mut along = 0.0;
    let mut kept = Vec::with_capacity(path.len());
    for (index, &point) in path.iter().enumerate() {
        if index > 0 {
            along += point.distance(path[index - 1]);
        }
        kept.push(point);
        if along >= reach {
            break;
        }
    }
    kept
}

/// Первое пересечение ломаных `a` и `b` по ходу `a` (а при равенстве — `b`):
/// точка и номера звеньев, на которых оно лежит.
fn first_crossing(a: &[Vec2], b: &[Vec2]) -> Option<(Vec2, [usize; 2])> {
    for i in 0..a.len().saturating_sub(1) {
        let (p, r) = (a[i], a[i + 1] - a[i]);
        let mut best: Option<(f32, f32, usize)> = None;
        for j in 0..b.len().saturating_sub(1) {
            let (q, s) = (b[j], b[j + 1] - b[j]);
            let denominator = r.perp_dot(s);
            if denominator.abs() < 1e-9 {
                continue;
            }
            let t = (q - p).perp_dot(s) / denominator;
            let u = (q - p).perp_dot(r) / denominator;
            if (0.0..=1.0).contains(&t)
                && (0.0..=1.0).contains(&u)
                && best.is_none_or(|(bt, ..)| t < bt)
            {
                best = Some((t, u, j));
            }
        }
        if let Some((t, _, j)) = best {
            return Some((p + r * t, [i, j]));
        }
    }
    None
}

/// Площадка плитки у бордюрной дуги угла между обочинами с газоном
/// ([`KERB_PAD_WIDTH`]): кольцевой сектор за дугой скругления дорог
/// (`halves`, `radius` — те же, что у него; дуга уже площадки — весь угол на
/// её ширину от обеих кромок) и по прямому хвосту [`KERB_PAD_RUN`] вдоль
/// края каждой дороги — не дальше дороги, которая её пересекает
/// ([`Arm::crossed`]). Куски — выпуклые, каждый веером из первой вершины.
pub(super) fn kerb_pad(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    radius: f32,
) -> Vec<Vec<Vec2>> {
    let Some(mut arc) = fillet_arc(node, first, second, halves, radius) else {
        return Vec::new();
    };
    // обе кромки кольца идут по газону во весь рост полосы — дуга тоже
    // режется мельче общего шага ([`KERB_ARC_SUBDIVISION`])
    arc.steps *= KERB_ARC_SUBDIVISION;
    // Ширина площадки у каждого луча — не шире его полосы тротуара, если
    // она есть: площадка в 3 м за тротуаром в 2.5 выходила в газон уступом
    // в полметра у конца хвоста и у начала дуги (Тула, Фёдора Смирнова у
    // Красноармейского, R1). Где тротуара нет, газон подходит к бордюру, и
    // площадка — во всю [`KERB_PAD_WIDTH`].
    let width = |sidewalk: Option<f32>| sidewalk.map_or(KERB_PAD_WIDTH, |s| s.min(KERB_PAD_WIDTH));
    let widths = [width(first.sidewalk[0]), width(second.sidewalk[1])];
    // кромка заходит под асфальт на `OVERLAP`, внутренняя дуга — того же
    // центра, на ширину площадки ближе к нему (от ширины у первого луча к
    // ширине у второго)
    let outer: Vec<Vec2> = arc.points(OVERLAP).collect();
    let sides = [arc.side_first, arc.side_second];
    let alongs = [arc.along_first, arc.along_second];
    let mut pieces: Vec<Vec<Vec2>> = if arc.radius > widths[0].max(widths[1]) {
        let inner: Vec<Vec2> = arc.points_between(-widths[0], -widths[1]).collect();
        outer
            .windows(2)
            .zip(inner.windows(2))
            .map(|(out, inn)| vec![out[0], out[1], inn[1], inn[0]])
            .collect()
    } else {
        // Дуга уже площадки: внутренняя сходилась в центр, и за ним, где у
        // луча нет хвоста (его прямой край кончился у касания), в угол
        // площадки проглядывал газон зубцом (Тула, Халтурина у Гоголевской).
        // Здесь площадка — весь угол на её ширину от обеих кромок: от дуги
        // по кромкам до задних краёв и до угла, где те сходятся.
        let determinant = sides[0].perp_dot(sides[1]);
        if determinant.abs() < 1e-6 {
            return Vec::new();
        }
        // на ширине площадки за каждой кромкой: `side_i · (p − угол) = W_i`
        let back = arc.corner
            + Vec2::new(
                widths[0] * sides[1].y - widths[1] * sides[0].y,
                widths[1] * sides[0].x - widths[0] * sides[1].x,
            ) / determinant;
        // по кромке — докуда она на ширину площадки от другой кромки
        let edge = |index: usize| {
            let reach = widths[1 - index] / alongs[index].dot(sides[1 - index]).max(1e-3);
            arc.corner + alongs[index] * reach.max(arc.tangent) - sides[index] * OVERLAP
        };
        let mut outline = vec![back, edge(0)];
        outline.extend(&outer);
        outline.push(edge(1));
        vec![outline]
    };
    // хвост — не дальше прямого края и не за дорожку, пересекающую улицу:
    // за ней снова газон, и хвост торчал за её лентой квадратом плитки в
    // газоне (Тула, Гоголевская у Халтурина, R10)
    for (on, along, side, room, width) in [
        (
            arc.on_first,
            arc.along_first,
            arc.side_first,
            first.run.min(first.crossed) - arc.t_first,
            widths[0],
        ),
        (
            arc.on_second,
            arc.along_second,
            arc.side_second,
            second.run.min(second.crossed) - arc.t_second,
            widths[1],
        ),
    ] {
        let run = KERB_PAD_RUN.min(room - arc.tangent);
        if run <= 0.0 {
            continue;
        }
        let kerb = on - side * OVERLAP;
        let back = on + side * width;
        pieces.push(vec![kerb, kerb + along * run, back + along * run, back]);
    }
    pieces
}

/// Дуга скругления ([`fillet`]): вершина угла краёв, точки касания, центр и
/// радиус — зажатый так, чтобы касательная не выходила за прямой край лучей.
pub(super) struct FilletArc {
    corner: Vec2,
    on_first: Vec2,
    on_second: Vec2,
    centre: Vec2,
    radius: f32,
    tangent: f32,
    /// Где от узла вдоль лучей край сошёлся с краем, м.
    t_first: f32,
    t_second: f32,
    /// Направления кромок от угла и нормали к ним наружу, в угол.
    along_first: Vec2,
    along_second: Vec2,
    side_first: Vec2,
    side_second: Vec2,
    from: Vec2,
    turn: f32,
    sweep: f32,
    steps: usize,
}

impl FilletArc {
    /// Точки дуги того же центра, на `extra` дальше от него, чем дуга
    /// скругления, — от касания с первым лучом до касания со вторым, концы
    /// включительно. При `extra == 0` это сама дуга, точка в точку.
    fn points(&self, extra: f32) -> impl Iterator<Item = Vec2> + '_ {
        let scale = (self.radius + extra) / self.radius;
        let ends = [self.on_first, self.on_second]
            .map(|on| on + (on - self.centre).normalize_or_zero() * extra);
        (0..=self.steps).map(move |step| match step {
            0 => ends[0],
            step if step == self.steps => ends[1],
            step => {
                let rotation =
                    Vec2::from_angle(self.turn * self.sweep * step as f32 / self.steps as f32);
                self.centre + rotation.rotate(self.from) * scale
            }
        })
    }

    /// Как [`Self::points`], но отступ от дуги плывёт линейно от `from` у
    /// касания с первым лучом до `to` у касания со вторым.
    fn points_between(&self, from: f32, to: f32) -> impl Iterator<Item = Vec2> + '_ {
        (0..=self.steps).map(move |step| {
            let share = step as f32 / self.steps as f32;
            let extra = from + (to - from) * share;
            let rotation = Vec2::from_angle(self.turn * self.sweep * share);
            self.centre + rotation.rotate(self.from) * ((self.radius + extra) / self.radius)
        })
    }
}

pub(super) fn fillet_arc(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    mut radius: f32,
) -> Option<FilletArc> {
    // Края, смотрящие друг на друга: у первого слева, у второго справа, — по
    // оси `u` с нормалью `n` и в клине наклонно: `e = u + n·наклон` на метр
    // оси (`Arm::slope`). Без наклона дуга под клином садилась на прямую
    // своей полуширины, мимо сужающейся кромки.
    let (axis_first, axis_second) = (first.direction, second.direction);
    let (normal_first, normal_second) = (axis_first.perp(), -axis_second.perp());
    let edge_first = axis_first + normal_first * first.slope[0];
    let edge_second = axis_second + normal_second * second.slope[1];
    let (along_first, along_second) = (edge_first.normalize(), edge_second.normalize());
    // ворота — по углу между осями, а не между краями: он и есть угол
    // перекрёстка, и его держат все правила узла (`bend`, `obtuse_corner`).
    // У кромки под клином край уходит от оси на atan(slope), и на устье
    // встречного клина — кромки равной ширины у узла, оси под 147°, — край
    // показывал 155°: ворота тупого клина, который сам откажет по наклону, и
    // угол оставался без куска (Одоевский путепровод, R37-04/3). Форма дуги —
    // по краям, как и раньше.
    if !(MIN_ANGLE..=MAX_ANGLE).contains(&ccw_angle(first, second)) {
        return None;
    }
    let angle = ccw_between(along_first, along_second);
    let (side_first, side_second) = (along_first.perp(), -along_second.perp());
    // угол, где встречаются края: halves.0·n₁ + t·e₁ = halves.1·n₂ + s·e₂, t и
    // s — метры по оси
    let rhs = normal_second * halves.1 - normal_first * halves.0;
    let determinant = -edge_first.perp_dot(edge_second);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-edge_second) / determinant;
    let s = edge_first.perp_dot(rhs) / determinant;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    let corner = node + normal_first * halves.0 + edge_first * t;

    let half_angle = angle / 2.0;
    // касательная не длиннее прямого края ленты: за следующей вершиной край
    // уже повернул, и дуга легла бы мимо
    let tangent = (radius / half_angle.tan())
        .min((first.run - t) * edge_first.length())
        .min((second.run - s) * edge_second.length());
    if tangent <= 0.0 {
        return None;
    }
    radius = tangent * half_angle.tan();
    if radius < MIN_RADIUS {
        return None;
    }
    let on_first = corner + along_first * tangent;
    let on_second = corner + along_second * tangent;
    let centre = corner + (along_first + along_second).normalize() * (radius / half_angle.sin());

    let sweep = PI - angle;
    let from = on_first - centre;
    let turn = from.perp_dot(on_second - centre).signum();
    let steps = arc_steps(radius, sweep).max(1);
    Some(FilletArc {
        corner,
        on_first,
        on_second,
        centre,
        radius,
        tangent,
        t_first: t,
        t_second: s,
        along_first,
        along_second,
        side_first,
        side_second,
        from,
        turn,
        sweep,
        steps,
    })
}
