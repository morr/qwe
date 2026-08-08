//! Решатель одного прогона: мелкая сетка связными списками, буферы толчков и
//! обход всех пар. Математику самой пары держит [`super::pairs`].

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::SeparationLab;
use super::pairs::{Pawn, anticipate, avoid_direction, shares, side_of, sidestep, yields};

/// «Конец цепочки» в связных списках мелкой сетки.
const NONE: u32 = u32::MAX;

/// Буферы прогона — в `Local`, чтобы steady state обходился без аллокаций.
/// Виден всему `movement`, а не только `separation`: тип стоит в сигнатуре
/// `separate_pawns`, которую регистрирует `movement/mod.rs`.
#[derive(Default)]
pub(in crate::movement) struct SeparationState {
    /// Кадр последнего прогона: гейт «не чаще раза в кадр».
    pub(super) last_frame: Option<u32>,
    /// Виртуальное время, накопленное тиками с последнего прогона.
    pub(super) pending_dt: f32,
    pub(super) pawns: Vec<Pawn>,
    /// Мелкая сетка соседей: голова цепочки по ячейке + связный список по
    /// индексам `pawns` — ни одного `Vec` на ячейку.
    pub(super) heads: HashMap<IVec2, u32>,
    pub(super) next: Vec<u32>,
    /// Пары этого прогона, каждая по разу: индексы плюс признак «уже
    /// перекрылись». Неперекрывшиеся сюда попадают только при включённом
    /// упреждении ([`SeparationLab::horizon`]) и только те, что в пределах
    /// горизонта. Собираются отдельным проходом, потому что толчок пары зависит
    /// от того, сколько соседей у её участников (см. `contacts`), а это
    /// известно только когда найдены все.
    pub(super) pairs: Vec<(u32, u32, bool)>,
    /// Сколько перекрытий у каждой пешки в этом прогоне.
    pub(super) contacts: Vec<u32>,
    /// Кто в этом прогоне упирался курсом в перекрытого соседа — источник
    /// [`SeparationHolds`].
    pub(super) held: Vec<bool>,
    pub(super) pushes: Vec<Vec2>,
    /// Толчки ТВЁРДОГО ЯДРА — отдельно от мягких, потому что ограничены они
    /// по-разному ([`SeparationLab::hard_core`]).
    pub(super) core_pushes: Vec<Vec2>,
    /// Сколько пар этого прогона уже перекрылись — остальные попали только в
    /// упреждение. Считается на месте, в проходе толчков.
    pub(super) overlapping: usize,
    /// Сумма направлений на перегородивших — источник [`SeparationBlock`].
    /// Копится ненормированной и нормируется в конце, как и `steers`.
    pub(super) blocks: Vec<Vec2>,
    /// Сумма сторон обхода по всем соседям — источник [`SeparationSteer`].
    /// Складывается ненормированной и нормируется один раз в конце: пешка,
    /// зажатая с двух сторон, должна получить их сумму, а не последнюю.
    pub(super) steers: Vec<Vec2>,
    /// Сколько виртуальных секунд подряд каждая пешка упирается курсом в чужое
    /// тело — единственное, что живёт МЕЖДУ прогонами: залипание это именно
    /// длительность. Пуста, пока [`SeparationLab::stuck_compress`] не тронут.
    pub(super) stuck: bevy::ecs::entity::EntityHashMap<f32>,
    /// Та же карта, собранная заново этим прогоном: ушедший из вьюпорта или
    /// прошедший затор выпадает из неё сам, без отдельной уборки.
    pub(super) stuck_next: bevy::ecs::entity::EntityHashMap<f32>,
}

pub(super) fn fine_cell(pos: Vec2, cell: f32) -> IVec2 {
    (pos / cell).floor().as_ivec2()
}

/// Детерминированное направление разведения точно совпавших позиций — хэш
/// [`PawnId`], тот же трюк, что `personal_spread` у веера бегства: пара держит
/// свою ось от прогона к прогону, а не дрожит случайной.
///
/// По `PawnId`, а не по `Entity`, по той же причине, что и там: индексы
/// сущностей после рестарта переиспользуются в другом порядке.
fn coincident_direction(pawn_id: u32) -> Vec2 {
    let hash = pawn_id.wrapping_mul(2654435761);
    let angle = (hash >> 8) as f32 / (u32::MAX >> 8) as f32 * std::f32::consts::TAU;
    Vec2::from_angle(angle)
}

/// Толчки всех пар текущего набора — чистая функция над буферами, без ECS
/// (тестируется напрямую). `fraction` — доля перекрытия, разрешаемая этим
/// прогоном; в `pushes` — суммарный сдвиг каждой пешки, ещё без клампа.
/// Ручки одного прогона: доля перекрытия, разрешаемая к снятию, плюс то, что
/// задаёт [`SeparationStyle`]. Отдельной структурой, чтобы у чистой функции не
/// росла череда безымянных `f32`.
#[derive(Clone, Copy)]
pub(super) struct Tuning {
    pub(super) fraction: f32,
    /// Виртуальное время, прошедшее с прошлого прогона: упреждение считает снос
    /// в м/с, а не в долях перекрытия.
    pub(super) dt: f32,
    pub(super) sidestep: f32,
    pub(super) cell: f32,
    pub(super) lab: SeparationLab,
}

/// Радиус тела с поправкой на давку: чем больше у пешки одновременных
/// перекрытий, тем сильнее она «ужимается» ([`SeparationLab::compress`]).
///
/// Зачем. Дистанция покоя 1.8 м — это личное пространство свободно идущего
/// человека, и в узком месте она физически недостижима: суммарной ширины
/// не хватает, коррекции всех пар складываются в затор, и поток встаёт.
/// Живая толпа в этом месте не встаёт, а прижимается. Ужимается ТОЛЬКО
/// величина коррекции: набор пар и, значит, `contacts` считаются по полному
/// радиусу — иначе загрузка зависела бы сама от себя.
///
/// При `compress = 0` возвращает радиус без изменений, то есть нынешнее
/// поведение.
fn squeezed_radius(pawn: &Pawn, contacts: u32, lab: &SeparationLab) -> f32 {
    let mut radius = pawn.radius;
    if lab.compress > 0.0 && lab.compress_at > 0.0 {
        let load = (contacts as f32 / lab.compress_at).min(1.0);
        radius *= 1.0 - lab.compress * load;
    }
    radius * (1.0 - lab.stuck_compress * stuck_load(pawn.stuck, lab))
}

/// Насколько пешка «залипла» — 0…1 по времени непрерывного упора
/// ([`SeparationLab::stuck_after`], [`SeparationLab::stuck_ramp`]).
///
/// Отдельно от числа контактов намеренно: сжатие по контактам получает вся
/// плотная толпа, в том числе стоящая и никуда не идущая, и её равновесие
/// оказывается перекрытым навсегда. Сжатие по времени упора получает только
/// тот, кто уже несколько раз подряд ткнулся в чужое тело и не прошёл, —
/// протискивание длится ровно столько, сколько длится тупик.
fn stuck_load(stuck: f32, lab: &SeparationLab) -> f32 {
    if lab.stuck_compress <= 0.0 {
        return 0.0;
    }
    let over = stuck - lab.stuck_after;
    if over <= 0.0 {
        return 0.0;
    }
    if lab.stuck_ramp <= 0.0 {
        return 1.0;
    }
    (over / lab.stuck_ramp).min(1.0)
}

/// Отпустило ли скольжение эту пешку: она упирается дольше, чем
/// [`SeparationLab::slide_release`], и запрет «не лезь в тело» с неё снят.
fn slide_released(pawn: &Pawn, lab: &SeparationLab) -> bool {
    lab.slide_release > 0.0 && pawn.stuck >= lab.slide_release
}

/// Дистанция покоя пары с поправкой на то, КТО в ней идёт.
///
/// Идущая мимо стоящей ужимается до [`SeparationLab::pass_squeeze`] — это
/// проход сквозь осевшую толпу, см. док ручки. Пара «оба стоят» и пара «оба
/// идут» не трогаются: плотность осевшей толпы и ширина потока — это то, ради
/// чего радиус тела вообще существует.
fn rest_distance(a: &Pawn, b: &Pawn, contacts: (u32, u32), lab: &SeparationLab) -> f32 {
    let full = squeezed_radius(a, contacts.0, lab) + squeezed_radius(b, contacts.1, lab);
    let a_walking = a.heading != Vec2::ZERO;
    let b_walking = b.heading != Vec2::ZERO;
    if lab.pass_squeeze < 1.0 && (a_walking != b_walking) {
        return full * lab.pass_squeeze;
    }
    full
}

pub(super) fn resolve_pushes(state: &mut SeparationState, tuning: Tuning) {
    let Tuning {
        fraction,
        dt,
        sidestep: sidestep_strength,
        cell: cell_size,
        lab,
    } = tuning;
    state.heads.clear();
    state.next.clear();
    state.pairs.clear();
    state.contacts.clear();
    state.contacts.resize(state.pawns.len(), 0);
    state.held.clear();
    state.held.resize(state.pawns.len(), false);
    state.pushes.clear();
    state.pushes.resize(state.pawns.len(), Vec2::ZERO);
    state.core_pushes.clear();
    state.core_pushes.resize(state.pawns.len(), Vec2::ZERO);
    state.steers.clear();
    state.steers.resize(state.pawns.len(), Vec2::ZERO);
    state.blocks.clear();
    state.blocks.resize(state.pawns.len(), Vec2::ZERO);
    state.overlapping = 0;

    // Насколько далеко имеет смысл смотреть на ещё не перекрывшегося соседа: за
    // горизонт `t` секунд самая быстрая пара сближается на сумму скоростей.
    // 0 при выключенном упреждении — тогда проход ниже не берёт ни одной лишней
    // пары и остаётся тем же, что был.
    //
    // Ячейка мелкой сетки от него РАСТЁТ: соседей ищут в 3 × 3 ячейках, и пара,
    // разнесённая дальше ячейки, туда просто не попадёт. Это и есть цена
    // упреждения — площадь просмотра растёт как квадрат горизонта (при 1.5 с и
    // 3.5 м/с ячейка 10.5 м против нынешних 2.4 м, то есть ~19× кандидатов на
    // пешку). Проверять этот счёт — работа стенда, а не догадки.
    let lookahead = if lab.horizon > 0.0 && lab.anticipation > 0.0 {
        let fastest = state
            .pawns
            .iter()
            .map(|pawn| pawn.speed)
            .fold(0.0f32, f32::max);
        lab.horizon * 2.0 * fastest
    } else {
        0.0
    };
    let cell_size = cell_size.max(lookahead);

    for (i, pawn) in state.pawns.iter().enumerate() {
        let head = state
            .heads
            .insert(fine_cell(pawn.position, cell_size), i as u32);
        state.next.push(head.unwrap_or(NONE));
    }

    // проход 1 — кто с кем перекрылся, кто с кем сближается, и сколько соседей
    // у каждого. `contacts` считает ТОЛЬКО перекрытия: он кормит гейт `alone` у
    // обхода и загрузку у сжатия радиуса, и упреждающие пары там не при чём
    for i in 0..state.pawns.len() {
        let a = state.pawns[i];
        let cell = fine_cell(a.position, cell_size);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(&head) = state.heads.get(&(cell + IVec2::new(dx, dy))) else {
                    continue;
                };
                let mut j = head;
                while j != NONE {
                    // пара встречается с обеих сторон — обрабатываем один раз
                    if j > i as u32 {
                        let b = state.pawns[j as usize];
                        let min_distance = a.radius + b.radius;
                        let gap_squared = (b.position - a.position).length_squared();
                        let overlapping = gap_squared < min_distance * min_distance;
                        if a.mobility + b.mobility > 0.0 {
                            if overlapping {
                                state.pairs.push((i as u32, j, true));
                                state.contacts[i] += 1;
                                state.contacts[j as usize] += 1;
                            } else if gap_squared < lookahead * lookahead {
                                state.pairs.push((i as u32, j, false));
                            }
                        }
                    }
                    j = state.next[j as usize];
                }
            }
        }
    }

    // проход 2 — толчки
    for pair in 0..state.pairs.len() {
        let (i, j, overlapping) = state.pairs[pair];
        let (a, b) = (state.pawns[i as usize], state.pawns[j as usize]);
        // сближающиеся, но ещё не сомкнувшиеся: только уклонение, и каждый
        // считает своё — доли пары тут ни при чём
        if !overlapping {
            state.pushes[i as usize] += anticipate(&a, &b, &lab, dt);
            state.pushes[j as usize] += anticipate(&b, &a, &lab, dt);
            if lab.steer > 0.0 {
                // руление копит ту же сторону, но взвешенную срочностью, а не
                // временем прогона: это доля курса, а не метры
                for (pawn, other, index) in [(&a, &b, i), (&b, &a, j)] {
                    if let Some((lateral, urgency)) = avoid_direction(pawn, other, &lab) {
                        state.steers[index as usize] += lateral * urgency;
                    }
                }
            }
            continue;
        }
        state.overlapping += 1;
        let min_distance = rest_distance(
            &a,
            &b,
            (state.contacts[i as usize], state.contacts[j as usize]),
            &lab,
        );
        let offset = b.position - a.position;
        let distance = offset.length();
        let direction = if distance > 1e-4 {
            offset / distance
        } else {
            coincident_direction(a.pawn_id)
        };
        // `max(0)` — из-за сжатия радиуса ([`squeezed_radius`]): в давке
        // дистанция покоя падает ниже фактической, и без клампа пара, которую
        // перекрытой посчитал проход 1, получила бы толчок ВНУТРЬ
        let overlap = (min_distance - distance).max(0.0) * fraction;
        let correction = direction * overlap;
        // Твёрдое ядро — отдельным слагаемым, и это не педантизм: мягкая часть
        // ограничена потолком скорости расталкивания (доли метра за прогон), а
        // встречная пара на полном ходу сближается за тот же прогон на впятеро
        // больше. Ядро, попавшее под общий потолок, работать не успевает — на
        // стенде это видно как сотни пар, сошедшихся ближе половины спрайта,
        // при формально включённом ядре. См. [`SeparationLab::hard_core`]
        let core_overlap = if lab.hard_core > 0.0 {
            ((a.radius + b.radius) * lab.hard_core - distance).max(0.0)
        } else {
            0.0
        };
        let (share_a, share_b) = shares(&a, &b, direction);
        // не «да/нет», а НАСКОЛЬКО в лоб: косинус между курсом и осью пары.
        // Скольжению ([`SeparationBlock`]) нужна именно величина — сосед,
        // задевший плечом, запрещает идти вперёд не так же, как вставший
        // ровно на пути, а сумма одинаковых «да» от полудюжины боковых даёт
        // равнодействующую точно по курсу и останавливает пешку там, где
        // прохода на самом деле никто не перекрывал
        let a_facing = a.heading.dot(direction);
        let b_facing = b.heading.dot(-direction);
        let a_frontal = a_facing > 0.0;
        let b_frontal = b_facing > 0.0;
        // Придержка — только упёршемуся в СТОЯЩЕГО или ВСТРЕЧНОГО: там напор
        // бесполезен — ходьба давит ровно против коррекции этой же пары, и оба
        // усилия взаимно гасятся навечно (см. [`SeparationHolds`]). Попутного
        // и поперечного соседа это не касается: первый вариант правила
        // придерживал за любое касание, и поток целиком полз на доле шага —
        // группа попутчиков душила сама себя.
        let a_blocked = a_frontal && (b.heading == Vec2::ZERO || b_frontal);
        let b_blocked = b_frontal && (a.heading == Vec2::ZERO || a_frontal);
        if a.human && a_blocked {
            state.held[i as usize] = true;
        }
        if b.human && b_blocked {
            state.held[j as usize] = true;
        }
        // запрет копится по тому же условию, что придержка, и у ВСЕХ, а не
        // только у людей: демон в погоне тоже не обязан входить в чужое тело
        if lab.slide > 0.0 {
            if a_blocked && !slide_released(&a, &lab) {
                state.blocks[i as usize] += direction * a_facing;
            }
            if b_blocked && !slide_released(&b, &lab) {
                state.blocks[j as usize] -= direction * b_facing;
            }
        }
        // Руление у СОМКНУВШЕЙСЯ пары. Условие то же, что у придержки, и это не
        // совпадение: придержка — «упёрся, перестань давить», руление — «упёрся,
        // возьми вбок». Упреждение здесь не помощник: тела уже соприкоснулись,
        // и обходить по его расчёту поздно. Сторона — своя правая, как у живых
        // пешеходов; ВСТРЕЧНЫМ она даёт противоположные стороны мира, то есть
        // ровно расхождение, а не вращение (вращение получалось у ТОЛЧКА, где
        // боковая добавка шла вместе с продольной коррекцией — здесь продольной
        // составляющей нет вовсе, курс только доворачивается).
        if lab.steer > 0.0 {
            if a_blocked && a.mobility > 0.0 {
                state.steers[i as usize] += side_of(&a, lab.left_share);
            }
            if b_blocked && b.mobility > 0.0 {
                state.steers[j as usize] += side_of(&b, lab.left_share);
            }
        }
        // обход — только встречным: догоняющего сзади ничто не держит, а
        // боковая добавка расползлась бы по всей очереди, идущей в одну сторону
        let opposed = a_frontal && b_frontal;
        // …и только паре, у которой нет других соседей. Обход разводит ДВОИХ,
        // симметрию которых больше сломать некому; в куче симметрию ломает сама
        // многотельная геометрия, а одинаковый разворот вправо у полусотни
        // перекрывшихся складывается в общее вращение — затор начинает крутиться
        // вместо того, чтобы рассасываться.
        //
        // [`SeparationLab::crowd_sidestep`] — ручка ровно на этот гейт: при 0
        // всё как сейчас, при >0 куча тоже обходит, но ослабленной долей.
        // Гипотеза, которую ей проверяют: боковой добавки в куче не хватает
        // именно там, где встречный поток обязан расслоиться на полосы, а
        // вращение — цена, которая, может быть, не наступает, пока добавка
        // мала.
        let alone = state.contacts[i as usize] == 1 && state.contacts[j as usize] == 1;
        let crowd = if alone { 1.0 } else { lab.crowd_sidestep };
        let (side_a, side_b) = if opposed && crowd > 0.0 {
            let strength = sidestep_strength * crowd;
            // уступает один: вправо уходят оба — и пара вращается, см. [`yields`]
            if yields(&a, &b) {
                (
                    sidestep(&a, direction, overlap * share_a, strength, lab.left_share),
                    Vec2::ZERO,
                )
            } else {
                (
                    Vec2::ZERO,
                    sidestep(&b, -direction, overlap * share_b, strength, lab.left_share),
                )
            }
        } else {
            (Vec2::ZERO, Vec2::ZERO)
        };
        state.pushes[i as usize] -= correction * share_a - side_a;
        state.pushes[j as usize] += correction * share_b + side_b;
        if core_overlap > 0.0 {
            let core = direction * core_overlap;
            state.core_pushes[i as usize] -= core * share_a;
            state.core_pushes[j as usize] += core * share_b;
        }
    }
}
