use bevy::prelude::*;
use rand::RngExt;

use super::in_recoil_cone;
use crate::human::components::{
    Human, HumanFirstWanderTag, HumanWanderTag, PanicRecoil, WanderHeading, WanderPause,
};
use crate::human::lod::FarTag;
use crate::map::osm::{MapData, PolyArea};
use crate::movement::{
    Movable, NeedsWanderTarget, SimPosition, assert_ready_to_pick, heading_towards, point_in_cone,
    request_wander_path,
};
use crate::navigation::Backend;
use crate::rng::{PawnId, RngDomain, SimRng, WanderIndex, WorldSeed};
use crate::settings::{
    HUMAN_WANDER_PAUSE, HUMAN_WANDER_PAUSE_SHARE, HUMAN_WANDER_RANGE,
    HUMAN_WANDER_TO_BUILDING_SHARE, RECOIL_MIN_ERRAND, WANDER_CONE,
};

/// Сколько зданий перебирается в поисках цели «по делам» в конусе курса;
/// если ни одно не попало — берётся ближайшее по направлению из выборки.
const WANDER_BUILDING_TRIES: usize = 8;

/// Точка, к которой идут «в этот дом»: вход из OSM, если он у дома размечен,
/// иначе случайная вершина контура. Выбор идёт от здания, а не от общего
/// списка входов, именно поэтому: размеченных в OSM входов на город сотни
/// (Тула — 491 на 7723 дома), и адресуйся пешки прямо к ним, двадцать тысяч
/// человек ходили бы по одним и тем же дверям.
fn building_target(building: &PolyArea, rng: &mut impl RngExt) -> Vec2 {
    let points = if building.entrances.is_empty() {
        &building.outer
    } else {
        &building.entrances
    };
    points[rng.random_range(0..points.len())]
}

/// Здание «по курсу»: из `WANDER_BUILDING_TRIES` случайных зданий
/// берётся первое, попавшее в конус вокруг `heading`; если ни одно не попало —
/// лучшее по направлению из выборки. Полный перебор 7500 зданий тут не нужен:
/// цель и так случайная, важно лишь не отправить пешку назад.
///
/// `ban` — запретный конус после паники (`PanicRecoil`). Кандидат в конусе или
/// ближе `RECOIL_MIN_ERRAND` отсеивается до всякого сравнения, то есть не
/// участвует и в запасном «лучшем по направлению»: именно этот запасной путь
/// раньше и мог вернуть здание почти строго назад, к демону. Вся выборка
/// отсеялась — `None`, вызывающий перебросит на следующем кадре.
fn pick_building_ahead(
    map: &MapData,
    rng: &mut impl RngExt,
    position: Vec2,
    heading: Vec2,
    ban: Option<Vec2>,
) -> Option<Vec2> {
    let cone_cos = WANDER_CONE.cos();
    let mut best: Option<(f32, Vec2)> = None;

    for _ in 0..WANDER_BUILDING_TRIES {
        let building = &map.buildings[rng.random_range(0..map.buildings.len())];
        let point = building_target(building, rng);
        if ban.is_some() && point.distance_squared(position) < RECOIL_MIN_ERRAND * RECOIL_MIN_ERRAND
        {
            continue;
        }
        let Some(direction) = (point - position).try_normalize() else {
            continue;
        };
        if ban.is_some_and(|ban| in_recoil_cone(direction, ban)) {
            continue;
        }
        let alignment = direction.dot(heading);
        if alignment >= cone_cos {
            return Some(point);
        }
        if best.is_none_or(|(best_alignment, _)| alignment > best_alignment) {
            best = Some((alignment, point));
        }
    }

    best.map(|(_, point)| point)
}

/// Куда человек хочет попасть на этом шаге — вся видовая политика мирного
/// блуждания; обвязка (заявка на путь, курс, снятие тегов, пауза) остаётся в
/// [`pick_wander_targets`].
///
/// 80% идят «по делам» к случайному зданию города (длинные маршруты, настоящая
/// нагрузка на pathfinding), 20% гуляют в 20–40 м от себя. Первая цель после
/// спавна — всегда прогулка поблизости (`is_first_wander`), первая после паники
/// (`ban`) — наоборот, всегда дальняя и не в запретном конусе.
///
/// `None` — цели на этом кадре нет: либо вся выборка зданий отсеялась запретом,
/// либо прогулка вышла в запретный конус. Вызывающий пропускает пешку целиком —
/// ни заявки, ни снятия тегов, ни перезарядки паузы.
///
/// **Число и порядок бросков — часть потока решений** (`CONTEXT.md`, «Decision
/// stream»): жребий 80/20 делается только там, где делался и раньше — короткое
/// замыкание `&&` не пускает его ни при пустом списке зданий, ни под запретом,
/// ни на первой цели; дальше идут броски одной выбранной ветки.
pub(super) fn choose_target(
    map: &MapData,
    rng: &mut SimRng,
    position: Vec2,
    heading: Vec2,
    ban: Option<Vec2>,
    is_first_wander: bool,
) -> Option<Vec2> {
    // после паники — только «по делам», причём это перебивает и бросок
    // 80/20, и `HumanFirstWanderTag`: тот существует, чтобы 20 000 пешек
    // не подали маршрут через весь город одним кадром, а паника на спавне
    // достаёт лишь толпу в 60 м от портала, и успокаиваются те вразнобой
    let to_building = !map.buildings.is_empty()
        && (ban.is_some()
            || (!is_first_wander && rng.random_range(0.0..1.0) < HUMAN_WANDER_TO_BUILDING_SHARE));

    if to_building {
        // «по делам»: вершина контура здания, лежащего по курсу — иначе
        // маршрут через весь город разворачивает пешку назад. Вся выборка
        // отсеялась запретом — `None`, новые восемь зданий следующим кадром;
        // сорваться на прогулку поблизости нельзя, это ровно то, от чего
        // человека и уводят
        return pick_building_ahead(map, rng, position, heading, ban);
    }

    // прогулка поблизости — в конусе вокруг курса
    let point = point_in_cone(rng, position, heading, WANDER_CONE, HUMAN_WANDER_RANGE);
    // под запретом сюда попадают только жители города без зданий: дальнего
    // маршрута там не существует, и прогулка с проверкой конуса — лучшее
    // доступное поведение. Проверять надо после клампа: у самого края карты он
    // и разворачивает направление
    if let Some(ban) = ban
        && (point - position)
            .try_normalize()
            .is_none_or(|direction| in_recoil_cone(direction, ban))
    {
        return None;
    }
    Some(point)
}

/// Пауза, которую человек выстоит на следующей цели: `HUMAN_WANDER_PAUSE_SHARE`
/// останавливаются на 2–10 с, остальные уходят дальше тем же кадром.
///
/// Бросок делается на цель, а не на человека: постоянно спешащая пятая часть
/// населения — это два разных сорта пешеходов, а нужен один, который иногда
/// останавливается. И бросается он заранее, при выборе цели, потому что это тот
/// же кадр, где пауза и так перезаряжается, — прибытие о ней ничего не знает.
///
/// Нулевая пауза срабатывает сразу: `Timer` в `Once` считает себя истёкшим при
/// первом же `tick`, если `elapsed >= duration`.
///
/// Дальнее блуждание (`human/lod.rs`) заряжает ту же паузу тем же броском —
/// «личность» пешки не зависит от яруса.
pub(crate) fn roll_wander_pause(rng: &mut impl RngExt) -> std::time::Duration {
    if rng.random_range(0.0..1.0) >= HUMAN_WANDER_PAUSE_SHARE {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f32(rng.random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1))
}

/// Радиус широкого кольца просеивания первого выхода гуляющего. Обычное
/// просеивание цели смотрит 8 соседей тайла (`Walkable::sift_target`), и на
/// плотном городе кольцо часто стоит целиком на доме — заявка уезжает на
/// следующий кадр. На шторме входа (20 000 первых выходов одним кадром) таких
/// переспрашиваний ~30%, и прогрев (`loading.rs::poll_warmup`) держит экран
/// загрузки, пока trickle этих заявок не иссякнет, — около десятка кадров.
/// Широкое кольцо ставит промахнувшуюся заявку тем же кадром; бросков потока
/// решений не делает (просеивание — геометрия), поэтому на выбор цели не
/// влияет — только на то, уедет ли она в этот кадр.
const FIRST_WANDER_SIFT_TILES: i32 = 4;

/// Мирное блуждание, один шаг скелета `movement/wander.rs`: отсев по состоянию
/// → пауза 2–10 с на каждом пятом прибытии (см. `roll_wander_pause`) → поток
/// решений → [`choose_target`] (вся политика «куда») → заявка на путь, курс,
/// снятие тегов и перезарядка паузы.
///
/// Каждый из четырёх `continue` — своё основание пропустить пешку до следующего
/// кадра, и ни один из них не двигает состояние вперёд.
#[allow(clippy::too_many_arguments)]
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    backend: Res<Backend>,
    map: Res<MapData>,
    seed: Res<WorldSeed>,
    determinism: Option<Res<crate::determinism::Determinism>>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            &mut WanderPause,
            &mut WanderHeading,
            &PawnId,
            &mut WanderIndex,
            &mut PanicRecoil,
            Has<HumanFirstWanderTag>,
        ),
        (
            With<Human>,
            With<HumanWanderTag>,
            // тег держит ровно `Idle` и `PathfindingError` (`needs_path`):
            // ставят его переходы `Movable`, тело цикла только стережёт этот
            // контракт под debug
            With<NeedsWanderTarget>,
            // дальние выбирают цель своим срезом (`human/lod.rs`) — без
            // заявки на поиск; под тумблером детерминизма `FarTag` не
            // существует, фильтр никого не режет
            Without<FarTag>,
        ),
    >,
) {
    // реплей ведёт свои заявки по тикам: широкий кольец — только живому режиму
    let deterministic = determinism.is_some_and(|mode| mode.0);
    let walkable = backend.walkable();

    for (
        entity,
        sim_position,
        mut movable,
        mut pause,
        mut heading,
        pawn_id,
        mut wander_index,
        mut recoil,
        is_first_wander,
    ) in &mut query
    {
        assert_ready_to_pick(&movable.state);

        pause.0.tick(time.delta());
        if !pause.0.is_finished() {
            continue;
        }

        // Поток заводится здесь, а не в начале итерации: до этой точки решение
        // ещё не принимается, а `next` сдвигает счётчик — тикающая пауза
        // прокручивала бы номера решений вхолостую, и число прокруток зависело
        // бы от частоты кадров.
        //
        // Засев — `(PawnId, номер решения)`, а не общий поток на систему и не
        // живой поток на пешке: выбор цели не должен зависеть ни от порядка
        // обхода запроса, ни от того, сколько выборок съело прошлое решение
        // этой же пешки
        let rng = &mut wander_index.next(seed.0, RngDomain::Human, pawn_id.0);

        // цель на этом кадре не выбралась — пешка ждёт следующего кадра, ничего
        // не сбрасывая: ни `PanicRecoil`, ни `HumanFirstWanderTag`, ни паузу
        let Some(target) = choose_target(
            &map,
            rng,
            sim_position.0,
            heading.0,
            recoil.0,
            is_first_wander,
        ) else {
            continue;
        };

        // первый выход не переспрашивает: на шторме входа переспрос ~30%
        // первых заявок растягивал прогрев на десяток кадров. Просеивание
        // цели для первого выхода — широким кольцом, живому режиму (см.
        // `sift_target_far`); неудача wide-кольца уезжает на следующий кадр,
        // как и раньше
        let target = if is_first_wander
            && !deterministic
            && walkable
                .sift_target(crate::grid::world_to_tile(target))
                .is_none()
        {
            walkable
                .sift_target_far(crate::grid::world_to_tile(target), FIRST_WANDER_SIFT_TILES)
                .map(crate::grid::tile_center)
                .unwrap_or(target)
        } else {
            target
        };

        let Some(target_tile) = request_wander_path(
            &mut commands,
            &walkable,
            entity,
            &mut movable,
            sim_position.0,
            target,
        ) else {
            continue;
        };
        // курс — по фактически выбранной цели, следующая пойдёт от него
        if let Some(direction) = heading_towards(sim_position.0, target_tile) {
            heading.0 = direction;
        }

        if is_first_wander {
            commands.entity(entity).remove::<HumanFirstWanderTag>();
        }
        // запрет живёт ровно до первой удачной цели — гасится записью на
        // месте: компонент постоянный (`#[require]` у `Human`), и снятие
        // командой перегоняло бы человека между архетипами на каждом выборе
        recoil.0 = None;

        // следующая пауза — уже после прибытия
        pause.0.set_duration(roll_wander_pause(rng));
        pause.0.reset();
    }
}
