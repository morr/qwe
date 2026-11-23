//! Камерный LOD людей — ярусы, срезы и дальнее поведение (Фаза 2 плана ×10,
//! `docs/population-x10-plan.md`).
//!
//! Вне экрана пешка живёт дёшево: блуждает и бежит по прямой без заявки на
//! поиск, решает раз в [`FAR_SLICE_TICKS`] тиков. У камеры — полный AI.
//! Дальние — [`FarTag`], срез фиксирован [`PawnId`](crate::rng::PawnId)
//! (маркер [`FarSlice<S>`]: срезная система обходит только свою восьмую
//! дальних, «нулевой пропуск» — никакого перебора всего населения).
//!
//! Правило границы — одно на весь проект, [`waits_for_camera`]: пешка Near,
//! пока диспетчер считает её видимой (зум внутрь города и кадр с запасом
//! `VIEW_MARGIN`). На городском зуме (≥ `WANDER_DISPATCH_MAX_ZOOM`) Near пуст
//! весь мир — дальние обязаны стать дешёвыми в тот же кадр, без выдержки.
//!
//! **Только живой режим** (`SimPipeline::Live`): под тумблером детерминизма
//! `FarTag` не существует вовсе — реплей идёт полным AI без камеры, и пин на
//! это стоит в `tests` этого модуля.

use bevy::prelude::*;
use rand::RngExt;

use crate::camera::Viewport;
use crate::demon::Demon;
use crate::diagnostics::{SIM_FAR_MS, SIM_PANIC_MS, SimMeasure};
use crate::human::behavior::FLEE_REPATH_PERIOD;
use crate::human::components::{
    CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag,
    Pace, PanicRecoil, WanderHeading, WanderPause,
};
use crate::human::decide::{
    FLEE_STEP, FleeAction, FleeSense, Threat, ThreatProbe, decide, escaped, flee_target,
};
use crate::human::systems::{in_recoil_cone, roll_wander_pause};
use crate::map::z::unit_z;
use crate::movement::{
    Movable, MovableState, MovableStateMovingTag, NeedsWanderTarget, PreviousSimPosition,
    SimPosition, StepOutcome, StepTuning, UrgentPath, VIEW_MARGIN, heading_towards, point_in_cone,
    step_along_path, waits_for_camera, wanderers_dispatched_at_zoom,
};
use crate::navigation::Backend;
use crate::rng::{PawnId, RngDomain, WanderIndex, WorldSeed};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_PANIC_RADIUS, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, HUMAN_WANDER_RANGE,
    RADIUS_HYSTERESIS, WANDER_CONE,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

#[cfg(test)]
mod tests;

/// Дальняя пешка: дешёвое поведение срезами, спрайт скрыт, из поштучных
/// проходов (интерполяция, выбор целей, паника, бегство, шаг) выпадает — всё
/// это делает [`far_slice`] на её срезе.
///
/// Архетипный маркер, а не значение: каждый поштучный проход отсекает его
/// фильтром `Without<FarTag>`, и под тумблером детерминизма (где дальних не
/// бывает) фильтр никого не режет. Зарегистрирован для BRP — `count FarTag`
/// и есть доля дальних в живом замере.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct FarTag;

/// Срез дальней пешки: `PawnId % FAR_SLICE_TICKS`, ставится на demote, снимается
/// на promote. Маркеров восемь, взаимоисключающих по построению (срез пишет
/// один сайт — [`slice_of`]), и срезная система фильтруется своим: на тике
/// `t` работает система `t % K` и обходит только свою восьмую дальних.
///
/// const-generic, а не значение в одном компоненте, потому что фильтр «равен
/// текущему тику» не архетипен — значение пришлось бы перебирать всем дальним
/// на каждом тике.
#[derive(Component, Default)]
pub struct FarSlice<const S: u8>;

/// Выдержка demote: сколько ещё запусков [`assign_lod_tiers`] пешка обязана
/// простоять за кадром, прежде чем уйти в дальние. Гасит шторм переносов
/// архетипов у кромки кадра при пане: пешка, входящая и выходящая из-за
/// кромки, сбрасывает счётчик, не перенесясь ни разу.
///
/// Состояние — значение, не наличие (урок `PanicRecoil`): ноль у свежей пешки
/// — расселение уводит толпу за кадр первым же проходом, не копя выдержку,
/// полный [`LodDwell::FULL`] — у побывавшей в кадре (после promote).
#[derive(Component, Reflect, Default, Clone, Copy, PartialEq, Debug)]
#[reflect(Component)]
pub struct LodDwell(pub u8);

impl LodDwell {
    /// Выдержка после пребывания в кадре. Запусков прохода назначения, а не
    /// тиков: проход идёт на движение камеры (и в режиме догона разгона), и
    /// выдержка «в тиках» при стоячей камере не тикала бы вовсе.
    pub const FULL: u8 = 64;
}

/// Период срезов дальних: шаг (и решение) раз в K тиков, `dt = K · тик`.
/// Скачок позиции за срез (0.35 м на 125 мс) глазу не виден — на зумах
/// дальнего слоя это доли пикселя (Фаза 5 плана).
pub const FAR_SLICE_TICKS: u64 = 8;

/// Сколько переносов между ярусами один проход [`assign_lod_tiers`] делает
/// каждой стороной. Разгон расселения уводит за кадр ~190k пешек — одним
/// проходом это сотни миллисекунд переноса архетипов; проход держит кадр,
/// а хвост разбирает в режиме догона (см. [`assign_lod_tiers`]). Прецедент
/// порций — `RESIZE_BATCH` у силуэтов.
pub const LOD_TRANSITION_BATCH: usize = 2048;

/// «Сердцебиение» прохода назначения: раз в столько кадров проход идёт и без
/// движения камеры. Без него стоячая камера спит с первой минуты мира: первый
/// проход случился ещё до того, как насос расселил первые партии, свежие
/// пешки остаются Near и копят очередь заявок, которую Фаза 2 обязана
/// опустошить. На сердцебиении работают обе стороны прохода: дальний ходок
/// заходит в кадр сам, срезами, — стоячей камере вернуть его некому, кроме
/// этого прохода. Стационарная цена копеечна — обход ближнего набора
/// (тысячи) и прямоугольник кадра по людской сетке (те же тысячи).
pub const LOD_HEARTBEAT: u32 = 4;

/// Срез пешки — по её номеру, не по месту в обходе: какие пешки обрабатывает
/// тик, не зависит от порядка спавна и смертей.
fn slice_of(pawn_id: u32) -> u8 {
    (pawn_id % FAR_SLICE_TICKS as u32) as u8
}

/// Поставить маркер среза — единственный писатель срезов, взаимоисключаемость
/// маркеров держится здесь.
fn insert_slice(commands: &mut Commands, entity: Entity, pawn_id: u32) {
    let mut entity = commands.entity(entity);
    match slice_of(pawn_id) {
        0 => entity.insert(FarSlice::<0>),
        1 => entity.insert(FarSlice::<1>),
        2 => entity.insert(FarSlice::<2>),
        3 => entity.insert(FarSlice::<3>),
        4 => entity.insert(FarSlice::<4>),
        5 => entity.insert(FarSlice::<5>),
        6 => entity.insert(FarSlice::<6>),
        _ => entity.insert(FarSlice::<7>),
    };
}

/// Снять дальний набор: ярус и срез. Зовут promote и `to_corpse` — труп
/// дальнего обязан вернуть видимый спрайт и не таскать маркеры, которые
/// никто, кроме promote, не снимет. `LodDwell` не трогает: выдержка нужна и
/// ближнему (её снимает только `to_corpse`, вместе с остальным людским).
pub fn remove_far(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<(
        FarTag,
        FarSlice<0>,
        FarSlice<1>,
        FarSlice<2>,
        FarSlice<3>,
        FarSlice<4>,
        FarSlice<5>,
        FarSlice<6>,
        FarSlice<7>,
    )>();
}

/// Назначение ярусов: promote в кадр, demote за кадр, живой режим, `Update`.
///
/// Запускается на движение камеры (сравнение `Viewport` в `Local`, как у
/// `resize_silhouettes_on_zoom`), на сердцебиение раз в
/// [`LOD_HEARTBEAT`] кадров (стоячая камера тоже ведёт обе стороны прохода:
/// дальний ходок заходит в кадр сам, срезами) и пока переносов больше
/// бюджета — каждый кадр, разбирая хвост разгона расселения: 190k ожидающих
/// demote пешек держали бы очередь заявок диспетчера — ту самую, которую
/// Фаза 2 обязана опустошить.
///
/// Стороны прохода:
/// - **demote** — обход ближнего набора (`Without<FarTag>`, тысячи): вышедшая
///   за кадр с запасом пешка вычитает выдержку, нулевая уходит в дальние.
///   Заявку (мирную или бегства) снимает `to_idle` — дальние заявок не подают;
///   бегству вдобавок снимается `UrgentPath`;
/// - **promote** — обход людской сетки прямоугольником кадра с запасом:
///   O(живые в кадре), не O(население). Дальний в кадре возвращается: спрайт
///   видим, `Transform` ставится из `SimPosition` (интерполяция не должна
///   тянуть из устаревшего `PreviousSimPosition`), беглец получает `UrgentPath`
///   назад и — если прямая оставила его на непроходимом — спасение: переезд
///   доходит и до людской сетки, а спрайт ставится уже из точки спасения;
///   настоящую заявку ближайший `flee` подаст сам, на следующем тике, из тех
///   же бросков потока решений, что и любой ближний (переход яруса не сдвигает
///   «личность» пешки — см. [`far_slice`]).
#[allow(clippy::too_many_arguments)]
pub fn assign_lod_tiers(
    mut commands: Commands,
    frame: Res<Viewport>,
    backend: Res<Backend>,
    // мутабельная: promote способен спасать беглеца из непроходимого, а
    // переезд обязан дойти до сетки (см. ветку promote ниже)
    mut human_grid: ResMut<SpatialGrid<Human>>,
    mut query: Query<
        (
            Entity,
            &mut SimPosition,
            &mut PreviousSimPosition,
            &mut Movable,
            &mut Transform,
            Has<FarTag>,
            Has<HumanFleeTag>,
            &PawnId,
            &mut LodDwell,
        ),
        (With<Human>, Without<crate::human::CorpseTag>),
    >,
    mut last_view: Local<Option<Viewport>>,
    mut backlog: Local<bool>,
    mut beats: Local<u32>,
) {
    // сердцебиение: без движения камеры проход всё равно идёт
    // раз в LOD_HEARTBEAT кадров (см. константу)
    *beats += 1;
    let heartbeat = (*beats).is_multiple_of(LOD_HEARTBEAT);
    if *last_view == Some(*frame) && !*backlog && !heartbeat {
        return;
    }
    *last_view = Some(*frame);
    // городской зум: мирных не диспетчит никто — Near обязан опустеть целиком,
    // в тот же проход, без выдержки (выдержка гасит шторм кромки кадра, а не
    // смену масштаба)
    let city_zoom = !wanderers_dispatched_at_zoom(frame.zoom);
    let mut budget = LOD_TRANSITION_BATCH;

    // ── demote: ближний набор ─────────────────────────────────────────────
    for (
        entity,
        sim_position,
        _previous,
        mut movable,
        _transform,
        far,
        fleeing,
        pawn_id,
        mut dwell,
    ) in &mut query
    {
        if far || budget == 0 {
            if far {
                continue;
            }
            break;
        }
        if !waits_for_camera(&frame, sim_position.0, false, VIEW_MARGIN) {
            // в кадре — выдержка заново: входящая и выходящая кромка не
            // переносится ни разу
            dwell.0 = LodDwell::FULL;
            continue;
        }
        // городской зум демote'ит сразу (выдержка гасит шторм кромки кадра,
        // а не смену масштаба); зум внутрь — после выдержки
        if !city_zoom && dwell.0 > 0 {
            dwell.0 -= 1;
        } else {
            budget -= 1;
            if let MovableState::Pathfinding(_) = movable.state {
                // заявка, поданная ближним, дальше не нужна: дальние заявок
                // не подают, а ждущая заявка — это очередь диспетчера
                movable.to_idle(entity, &mut commands, false);
            }
            if fleeing {
                commands.entity(entity).remove::<UrgentPath>();
            }
            commands.entity(entity).insert((FarTag, Visibility::Hidden));
            insert_slice(&mut commands, entity, pawn_id.0);
        }
    }

    // ── promote: кадр по людской сетке ────────────────────────────────────
    // на каждом проходе системы, не только на движении камеры: дальний
    // ходок заходит в кадр сам, срезами, и стоячая камера возвращает его
    // сердцебиением. На городском зуме Near не существует — возвращать
    // некого и незачем
    if !city_zoom && budget > 0 {
        let view = frame.with_margin(VIEW_MARGIN);
        let walkable = backend.walkable();
        let mut promoted = 0;
        // спасённые при promote: сетка занята общим обходом кадра, отчёт о
        // телепортах копится здесь и уходит после обхода
        let mut rescued = Vec::new();
        human_grid.for_each_in_rect(view.min(), view.max(), |entity| {
            if promoted >= budget {
                return;
            }
            let Ok((
                _entity,
                mut sim_position,
                mut previous,
                mut movable,
                mut transform,
                far,
                fleeing,
                _pawn_id,
                mut dwell,
            )) = query.get_mut(entity)
            else {
                return;
            };
            if !far {
                return;
            }
            promoted += 1;
            remove_far(&mut commands, entity);
            commands.entity(entity).insert(Visibility::Visible);
            dwell.0 = LodDwell::FULL;
            if fleeing {
                // срочность — вместе с паникой, как у всякого ближнего
                // беглеца; заявку подаст `flee` ближайшим тиком
                // (`needs_path`), из тех же бросков потока
                commands.entity(entity).insert(UrgentPath);
                // прямая бегства не обязана знать проходимость ПОД НОГАМИ
                // (полимеш строже сетки в углах) — та же страховка, что у
                // `rescue_from_impassable`
                if !walkable.allows(sim_position.0)
                    && let Some(free) = walkable.nearest_free_point(sim_position.0)
                {
                    rescued.push((entity, free));
                    sim_position.0 = free;
                    movable.to_idle(entity, &mut commands, false);
                }
            }
            // оба конца интерполяции — из сим-позиции (у спасённой — уже из
            // точки спасения): за время в дальних `PreviousSimPosition`
            // устарел на срез, и лерп дёрнул бы пешку
            transform.translation = sim_position.0.extend(unit_z(sim_position.0.y));
            previous.0 = sim_position.0;
        });
        budget -= promoted;
        // спасение — писатель `SimPosition` мимо шага и среза, и сетка людей
        // обязана об нём узнать, как от трёх остальных сайтов спасения
        // (`movement::systems::rescue_trapped_entities` и два на приёме
        // ответов): пропущенный переезд оставил бы пешку кандидатом в
        // покинутой ячейке — для захвата демонов, сбора паники и следующего
        // promote — пока она не пересечёт границу снова
        for (entity, free) in rescued {
            human_grid.insert(entity, free);
        }
    }

    // хвост разгонной волны: работаем и без движения камеры, пока есть
    // незабранные переходы
    *backlog = budget == 0;
}

/// Один срез дальних: поведение и шаг всех дальних пешек среза `S`.
///
/// Зарегистрирована восемь раз (по маркеру среза), гейт `tick % K == S`
/// оставляет активной одну копию на тик — пешка получает своё решение и свой
/// шаг раз в [`FAR_SLICE_TICKS`] тиков, с `dt = K · тик` (движение за срез
/// оплачивается одним вызовом `step_along_path`, а не восемью).
///
/// Дальнее поведение — те же чистые правила, что у ближних, с тремя
/// отличиями, и все три про цену:
/// - **блуждание** — только прогулка в конусе курса ([`point_in_cone`]), без
///   «дел» у зданий; цель годится, если до неё прямая
///   ([`line_of_sight`](Backend::walkable)) — путь `[target]`, без заявки на
///   поиск; нет прямой — переброс на следующем срезе;
/// - **паника** — грубая проверка занятости ячеек сетки демонов
///   (`any_in_cells_around`, O(1)); переход в бегство пишет те же записи, что
///   ближний `panic`, минус `UrgentPath` и минус заявка;
/// - **бегство** — та же чистая [`decide`] и те же броски потока решений в том
///   же порядке (период, шаг): «личность» пешки не зависит от того, в каком
///   ярусе её застала паника. Применение — вектор от угрозы и путь-прямая,
///   как у блуждания. Спасение за краем карты — та же проверка `escaped`,
///   на срезе: инвариант телеметрии (`escaped`) не различает ярусов.
///
/// `WanderIndex` дальнего сдвигается только на решениях — срез, в котором
/// решения нет (идёт по прямой, пауза не кончилась), потока не касается, как и
/// у ближних.
#[allow(clippy::too_many_arguments)]
pub fn far_slice<const S: u8>(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    time: Res<Time>,
    backend: Res<Backend>,
    demons: Res<SpatialGrid<Demon>>,
    // `Without<Human>` разводит этот запрос с мутабельным `SimPosition`
    // дальних доказуемо (B0001: `With<Demon>` alone не исключает `Human`);
    // демоны — не люди, проектный факт
    demon_positions: Query<(&SimPosition, Option<&PawnId>), (With<Demon>, Without<Human>)>,
    style: Res<HumanStyle>,
    seed: Res<WorldSeed>,
    mut telemetry: ResMut<Telemetry>,
    mut human_grid: Option<ResMut<SpatialGrid<Human>>>,
    mut query: Query<
        (
            Entity,
            &mut SimPosition,
            &mut PreviousSimPosition,
            &mut Movable,
            &mut WanderPause,
            &mut WanderHeading,
            &Pace,
            &PawnId,
            &mut WanderIndex,
            &mut PanicRecoil,
            Option<&mut FleeRepath>,
            // теги состояний взаимоисключающие (их ставят парами `panic` и
            // успокоение), мирная ветка ниже — просто «не бегущая»: 15 полей —
            // потолок кортежа `Query`, и `Has<HumanWanderTag>` здесь лишний
            Has<HumanFleeTag>,
            Has<HumanFirstWanderTag>,
            Has<NeedsWanderTarget>,
            Has<MovableStateMovingTag>,
        ),
        (With<Human>, With<FarTag>, With<FarSlice<S>>),
    >,
) {
    let _measure = SimMeasure::new(&mut diagnostics, &SIM_FAR_MS);
    let walkable = backend.walkable();
    // время среза: один шаг поглощает K тиков сразу — и таймеры тикают тем же
    // срезным временем (репат, пауза)
    let dt = time.delta_secs() * FAR_SLICE_TICKS as f32;
    let dt_duration = time.delta().mul_f32(FAR_SLICE_TICKS as f32);
    let rest = crate::movement::BodyScale::HUMAN.rest(style.body_radius);

    for (
        entity,
        mut sim_position,
        mut previous,
        mut movable,
        mut pause,
        mut heading,
        pace,
        pawn_id,
        mut wander_index,
        mut recoil,
        mut repath,
        fleeing,
        first_wander,
        needs_target,
        walking,
    ) in &mut query
    {
        // ── поведение: бегство или мирное ─────────────────────────────────
        if fleeing && let Some(repath) = repath.as_deref_mut() {
            // таймер перепрокладки тикает срезным временем: периоды те же,
            // что у ближнего, просто тикает раз в K тиков. Паузу бегущий не
            // тикает — её тикает выбор цели, как у ближних
            repath.0.tick(dt_duration);
            let sense = FleeSense {
                position: sim_position.0,
                pawn_id: pawn_id.0,
                // за дальней пешкой не гоняется никто: демоны, чья жертва
                // ушла за кадр, рано или поздно `LostTarget`/`GaveUp`
                chased: false,
                repath_due: repath.0.just_finished(),
                needs_path: movable.state.needs_path(),
            };
            let action = decide(&sense, |probe| match probe {
                ThreatProbe::Cells => {
                    if demons
                        .any_in_cells_around(sim_position.0, HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS)
                    {
                        Threat::Near
                    } else {
                        Threat::None
                    }
                }
                ThreatProbe::Nearest => demons
                    .nearest_in_range(
                        sim_position.0,
                        HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS,
                        |demon| crate::spatial::pawn_position(&demon_positions, demon),
                        |demon| crate::spatial::pawn_order(&demon_positions, demon),
                    )
                    .map_or(Threat::None, |(_, position)| Threat::At(position)),
            });

            match action {
                FleeAction::CalmDown => {
                    movable.speed = pace.speed(HUMAN_WALK_SPEED, style.spread);
                    movable.to_idle(entity, &mut commands, false);
                    pause.0.set_duration(std::time::Duration::from_secs_f32(
                        wander_index
                            .next(seed.0, RngDomain::Human, pawn_id.0)
                            .random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1),
                    ));
                    pause.0.reset();
                    // `UrgentPath` у дальнего не было — снимать нечего
                    commands
                        .entity(entity)
                        .remove::<(HumanFleeTag, FleeRepath)>()
                        .insert(HumanWanderTag);
                }
                FleeAction::Hold => {}
                FleeAction::Flee { away, ban } => {
                    // запрет — значением на месте, как у ближнего: компонент
                    // постоянный, перенос архетипа не нужен
                    recoil.0 = Some(ban);
                    heading.0 = away;
                    let step = wander_index
                        .next(seed.0, RngDomain::Human, pawn_id.0)
                        .random_range(FLEE_STEP.0..FLEE_STEP.1);
                    let target = flee_target(sim_position.0, away, step);
                    direct_path(
                        &walkable,
                        entity,
                        &mut movable,
                        sim_position.0,
                        target,
                        &mut commands,
                    );
                    // нет прямой — переброс: следующий срез спросит угрозу
                    // заново (`needs_path` у стоящего держит тик решения)
                }
            }
        } else {
            // onset паники дальних — НЕ здесь: прямой опрос сетки демонов
            // каждой мирной пешкой среза стоил бы 25k × O(1) на тик — 4.5 мс
            // на 200k по живому замеру звена №47. Панику дальним приносит
            // инвертированный сбор [`far_onset`]: демоны × соседи, как у
            // ближнего `panic`
            if needs_target {
                pause.0.tick(dt_duration);
                if pause.0.is_finished() {
                    let rng = &mut wander_index.next(seed.0, RngDomain::Human, pawn_id.0);
                    let target = point_in_cone(
                        rng,
                        sim_position.0,
                        heading.0,
                        WANDER_CONE,
                        HUMAN_WANDER_RANGE,
                    );
                    // первая цель после паники не смотрит на демона — та же
                    // проверка, что у ближнего города без зданий
                    let banned = recoil.0.is_some_and(|ban| {
                        (target - sim_position.0)
                            .try_normalize()
                            .is_none_or(|direction| in_recoil_cone(direction, ban))
                    });
                    if !banned
                        && let Some(target_tile) = direct_path(
                            &walkable,
                            entity,
                            &mut movable,
                            sim_position.0,
                            target,
                            &mut commands,
                        )
                    {
                        if let Some(direction) = heading_towards(sim_position.0, target_tile) {
                            heading.0 = direction;
                        }
                        if first_wander {
                            commands.entity(entity).remove::<HumanFirstWanderTag>();
                        }
                        recoil.0 = None;
                        pause.0.set_duration(roll_wander_pause(rng));
                        pause.0.reset();
                    } else {
                        // запретная или непрямая цель — остылка, а не холостой
                        // переброс: пешка в кармане двора билась бы в стену
                        // каждым срезом (бросок + луч по сетке), и в плотной
                        // застройке таких стоящих — большинство; 0.5–1 с
                        // простоя — и цена, и вид «переминается на месте»
                        pause.0.set_duration(std::time::Duration::from_secs_f32(
                            rng.random_range(0.5..1.0),
                        ));
                        pause.0.reset();
                    }
                }
            } else {
                // идёт по своей прямой — паузу тикать незачем: она перезаряжается
                // на выборе цели и досчитает первым же срезом после прибытия
            }
        }

        // ── шаг среза ─────────────────────────────────────────────────────
        if !walking {
            continue;
        }
        let was_at = sim_position.0;
        let mut position = sim_position.0;
        let outcome = step_along_path(
            &mut movable,
            &mut position,
            dt,
            Default::default(),
            StepTuning {
                rest,
                // модификаторов расталкивания у дальних нет (оно не выходит
                // за кадр demote'а), так что все три ручки не читаются
                arrive_slack: 0.0,
                steer_release: 0.0,
                slide: 0.0,
            },
            &walkable,
        );
        previous.0 = was_at;
        sim_position.0 = position;
        match outcome {
            StepOutcome::Moved => {}
            StepOutcome::Arrived {
                destination_reached,
            } => {
                previous.0 = sim_position.0;
                movable.to_idle(entity, &mut commands, destination_reached);
            }
            // докат упёрся или стоял — срезать тег: следующий срез решит заново
            StepOutcome::Halted => {
                previous.0 = sim_position.0;
                commands.entity(entity).remove::<MovableStateMovingTag>();
            }
        }

        // сетка людей ведётся и для дальних: паника у демонов, жертвы
        // приобретения и сам promote спрашивают её по позиции
        if let Some(grid) = human_grid.as_mut() {
            grid.moved(entity, was_at, sim_position.0);
        }

        // спасение за краем — та же проверка, на срезе: телеметрия не
        // различает ярусов
        if fleeing && escaped(sim_position.0) {
            commands.entity(entity).despawn();
            telemetry.escaped += 1;
        }
    }
}

/// Onset паники дальних — инвертированный сбор, зеркально ближнему `panic`:
/// не каждая мирная пешка опрашивает сетку демонов (25k × O(1) на тик —
/// 4.5 мс на 200k, живой замер звена №47), а каждый демон собирает соседей
/// по сетке людей. Дальние мирные попадают в тот же сбор дважды-нет — ближний
/// `panic` отсекает их `Without<FarTag>`, здесь наоборот `With<FarTag>`; запись
/// перехода та же, что у ближнего, минус `UrgentPath` (заявки у дальних нет).
#[allow(clippy::too_many_arguments)]
pub fn far_onset(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    humans: Res<SpatialGrid<Human>>,
    style: Res<HumanStyle>,
    demons: Query<&SimPosition, With<Demon>>,
    wanderers: Query<
        &SimPosition,
        (
            With<Human>,
            With<HumanWanderTag>,
            With<FarTag>,
            Without<CorpseTag>,
        ),
    >,
    seed: Res<WorldSeed>,
    mut movables: Query<(&mut Movable, &Pace, &PawnId, &mut WanderIndex)>,
) {
    let _measure = SimMeasure::new(&mut diagnostics, &SIM_PANIC_MS);
    // дедуп между демонами: дальнего в двух радиусах паникуем один раз
    let mut panicked: bevy::platform::collections::HashSet<Entity> =
        bevy::platform::collections::HashSet::default();
    for demon_position in &demons {
        humans.for_each_in_cells_around(demon_position.0, HUMAN_PANIC_RADIUS, |human| {
            if panicked.contains(&human) {
                return;
            }
            let Ok(human_position) = wanderers.get(human) else {
                return;
            };
            if human_position.0.distance_squared(demon_position.0)
                <= HUMAN_PANIC_RADIUS * HUMAN_PANIC_RADIUS
            {
                panicked.insert(human);
            }
        });
    }
    for &entity in &panicked {
        // промах запроса невозможен по построению (сущность из `wanderers`);
        // `expect`, а не молчаливый дефолт — тот же довод, что у `panic`
        let (mut movable, pace, pawn_id, mut wander_index) = movables
            .get_mut(entity)
            .expect("у дальнего гуляющего нет Movable/Pace/PawnId/WanderIndex");
        movable.speed = pace.speed(HUMAN_FLEE_SPEED, style.spread);
        let period = wander_index
            .next(seed.0, RngDomain::Human, pawn_id.0)
            .random_range(FLEE_REPATH_PERIOD);
        let mut repath = FleeRepath::default();
        repath
            .0
            .set_duration(std::time::Duration::from_secs_f32(period));
        repath.0.almost_finish();
        commands
            .entity(entity)
            .remove::<HumanWanderTag>()
            // без `UrgentPath`: заявки у дальнего нет и не будет, а срочность
            // без заявки — маркер, который никто не прочтёт
            .insert((HumanFleeTag, repath));
    }
}

/// Путь-прямая: цель видима с текущей точки — путь из неё одной, состояние
/// `Moving`, без заявки на поиск. Возвращает тайл цели, `None` — прямой нет
/// (вызывающий оставляет пешку на переброс следующего среза).
#[allow(clippy::too_many_arguments)]
fn direct_path(
    walkable: &crate::navigation::Walkable,
    entity: Entity,
    movable: &mut Movable,
    from: Vec2,
    target: Vec2,
    commands: &mut Commands,
) -> Option<IVec2> {
    if !walkable.line_of_sight(from, target) {
        return None;
    }
    let tile = crate::grid::world_to_tile(target);
    movable.to_moving(tile, [target].into(), entity, commands);
    Some(tile)
}
