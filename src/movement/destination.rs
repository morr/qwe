//! Слоты назначения (destination slots): у каждой пешки своя конечная точка.
//!
//! **Слот** — блок `k × k` навтайлов, зарезервированный одной пешкой под цель.
//! Заявка ([`DestinationClaim`]) выдаётся при выборе цели; занятый слот уводит
//! цель на ближайший свободный — кольцевым поиском наружу.
//!
//! Зачем. Без заявок несколько пешек целятся в одну точку, а занять её может
//! одна. Выхода из этого у симуляции нет: точка пути снимается, только когда
//! шаг тика накрывает остаток дистанции (`systems::move_moving_entities`), а
//! упёршуюся пешку расталкивание отбрасывает назад ровно настолько, насколько
//! она шагнула, — перекрытие встаёт на равновесие
//! `HUMAN_WALK_SPEED / (SEPARATION_RATE · доля)` = 0.7 м и держится вечно.
//! Прибытие же требует точного совпадения тайла. В итоге пешки либо наматывают
//! круги вокруг общей цели, либо стоят колом и дрожат на шаг за кадр, либо,
//! дойдя, никогда не замирают.
//!
//! # Почему блок тайлов, а не тайл и не своя решётка
//!
//! Цель здесь везде ключуется навтайлом — `MovableState`,
//! [`PathfindingRequest::end_tile`], фильтр устаревших ответов, тест прибытия,
//! да и полигональный меш ищет в `tile_center(end_tile)`. Точка, не являющаяся
//! центром тайла, — это точка, до которой пешка не может «дойти» по
//! определению, поэтому отдельной решётки с дробным шагом тут быть не может.
//!
//! Но и «один хозяин на тайл» не годится: гарантия «осевшая толпа не
//! перекрывается» держалась бы, только пока дистанция покоя `2 · radius`
//! не больше тайла. Её ломает и `NavtileBase::M1` (тайл 1 м против покоя 1.8 м),
//! и ползунок [`HumanStyle::body_radius`], который задуман изменчивым.
//!
//! Отсюда блок: `k = ceil(дистанция покоя / navtile_size())`, слот —
//! `tile.div_euclid(k)`, цель слота — строго ЦЕНТРАЛЬНЫЙ тайл блока
//! (`slot · k + k / 2`). Цели соседних слотов отстоят ровно на `k · navtile`,
//! то есть не меньше дистанции покоя, при любой комбинации настроек, и всё
//! это в целых числах — без пересчёта координат и накопления ошибки.
//!
//! Центральный тайл фиксирован намеренно. Если позволить блоку выбирать в себе
//! любой проходимый тайл, два соседних блока выберут смежные углы и сойдутся на
//! расстояние в один тайл — ровно та беда, от которой блок и заводился. Плата:
//! блок с непроходимым центром не используется вовсе, и шаг округляется вверх
//! до кратного тайлу (до ~2× дистанции покоя в худшем случае), то есть толпа
//! местами паркуется просторнее необходимого.
//!
//! Заявка живёт от выбора цели до СЛЕДУЮЩЕГО выбора; на прибытии она
//! намеренно не снимается — стоящая на своём слоте пешка и есть та занятость,
//! которую слоты моделируют.
//!
//! # Кого слоты не касаются
//!
//! - **погоня** (`DemonChaseTag`) — общая цель у преследователей это механика
//!   («клещи», `MAX_CHASERS_PER_TARGET`), сдвиг сломал бы и её, и передачу в
//!   бросок;
//! - **бегство** (`HumanFleeTag`) — цели переприкладываются раз в 0.7–1.2 с,
//!   разведены веером `personal_spread` и часто лежат за краем карты. Заявки на
//!   них — сотни кольцевых поисков в секунду впустую, а давка при панике всё
//!   равно определяется не целями, а шириной улицы.
//!
//! # Детерминизм
//!
//! В отличие от расталкивания слоты работают в ОБОИХ режимах: это симуляция, а
//! не косметика. Поэтому здесь нет ни гейта по камере (незаявленные кучи
//! копились бы за кадром, и камера приезжала бы ровно в них), ни зависимости от
//! `FrameCount`. Итерации по `HashMap` в выход не попадают — только точечные
//! `get`/`insert`/`remove`, — а порядок назначения задаётся сортировкой пакета
//! по `(вид, PawnId)`, как в `apply_pathfinding_results`.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::grid::tile_center;
use crate::human::HumanStyle;
use crate::movement::components::{Movable, MovableState, PathfindingRequest};
use crate::navigation::ArcNavmesh;
use crate::settings::{CLAIM_SEARCH_METERS, navtile_size};

/// Слот, зарезервированный этой пешкой под конечную точку (координаты решётки
/// слотов, не тайлов). Источник истины; [`DestinationClaims`] — обратный индекс.
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Component)]
pub struct DestinationClaim(pub IVec2);

/// Докуда ищется свободный слот, м. Ресурс, а не константа: у величины нет
/// правильного значения, есть компромисс — мало, и хвост большой толпы остаётся
/// без слотов (уходит в общую точку); много, и цель уезжает от задуманной
/// поведением так далеко, что это уже другая цель. Подбирается ползунком на
/// живой толпе (`examples/demos/crowd_demo.rs`), как радиус расталкивания.
///
/// Намеренно НЕ `SettingsGroup`: в игре величина не имеет смысла как вкус
/// пользователя, а демо, подвинув её, не должно править конфиг игры.
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
pub struct SlotSearch(pub f32);

impl Default for SlotSearch {
    fn default() -> Self {
        Self(CLAIM_SEARCH_METERS)
    }
}

/// Экспериментальные ручки слотов — то же, чем [`SeparationLab`] служит
/// расталкиванию: стенд (`examples/demos/crowd_demo.rs`) их перебирает, игра
/// живёт на дефолте, и дефолт воспроизводит нынешнее поведение точь-в-точь.
///
/// [`SeparationLab`]: crate::movement::SeparationLab
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, Default)]
pub struct SlotLab {
    pub matching: SlotMatching,
    /// Насколько НАВТАЙЛОВ шире дистанции покоя ставится шаг решётки слотов
    /// (0 — как сейчас, вплотную).
    ///
    /// Зачем. Шаг решётки — ровно дистанция покоя, округлённая вверх до тайла:
    /// осевшая толпа стоит впритык, и пешке, чей слот ещё внутри, физически
    /// негде между осевшими пройти — её ширина 1.8 м, а просвет между двумя
    /// соседями 2.0 м. Каждый входящий поэтому обязан протолкнуться, и то, что
    /// толпа уже осела, ей не помогает.
    ///
    /// Лишний тайл разводит слоты на 4 м, и просвет становится проходимым. За
    /// это платят площадью: та же толпа садится на вчетверо большее пятно —
    /// то есть уходит от центра дальше. Что из этого дороже, решает замер, а не
    /// рассуждение.
    pub slack: i32,
    /// Ближе какого расстояния до цели выдаётся слот, м. 0 — слот выдаётся при
    /// ВЫБОРЕ цели, как сейчас.
    ///
    /// Зачем. Слот назначается на СТАРТЕ, а занимается на ФИНИШЕ: пешка, которой
    /// достался слот в глубине толпы, идёт к нему полтора десятка секунд и
    /// приходит к моменту, когда её место обстроено теми, кому достались
    /// наружные слоты — наружные ближе, и дошли они раньше. Пробиться внутрь
    /// она может только сквозь тела; отсюда и слипшийся ком, и карусель вокруг
    /// него (`tools/separation_slots_lab/REPORT.md`, раздел 5).
    ///
    /// Живая толпа так не делает: пришедший первым проходит вглубь, пришедший
    /// последним встаёт с краю. Отложенная выдача воспроизводит ровно это —
    /// пешка идёт к цели как к ТОЧКЕ, а войдя в эту окрестность, забирает
    /// ближайший к цели свободный слот. Все занятые к этому моменту лежат
    /// глубже неё, значит идти сквозь осевших не нужно ни разу.
    pub claim_at: f32,
    /// На сколько метров осевшую пешку должно оттолкнуть от её слота, чтобы она
    /// пошла обратно (0 — не возвращается, как сейчас).
    ///
    /// Зачем. Дошедшую пешку толкают проходящие мимо, она сходит со слота — и
    /// вернуться ей нечем: маршрут исчерпан, новую цель в воронке никто не
    /// выдаёт. Дальше возмущение расходится по толпе: сошедшая перекрывается с
    /// соседкой, та отъезжает, и так до края. На стенде это `idle_drift` в
    /// сотни метров ПОСЛЕ того, как последняя пешка встала, — то есть второй
    /// критерий («никто не двигается») не выполняется никогда, а третий растёт
    /// из ничего.
    ///
    /// В игре роль обратной связи играет то, что блуждающий рано или поздно
    /// выберет новую цель; у толпы, стоящей у портала, этого нет.
    pub regroup: f32,
}

impl Default for SlotLab {
    fn default() -> Self {
        Self {
            // пакетное назначение — дефолт с тех пор, как замер показал, что оно
            // улучшает три критерия из четырёх и не стоит ничего
            // (`tools/separation_slots_lab/REPORT.md`, 3.1)
            matching: SlotMatching::Batch,
            slack: 0,
            claim_at: 0.0,
            regroup: crate::settings::SLOT_REGROUP_METERS,
        }
    }
}

/// Как пакет пешек, идущих в ОДНУ И ТУ ЖЕ точку, разбирает свободные слоты.
///
/// Ручка стенда, а не вкус пользователя: у обоих режимов один и тот же выход
/// (каждому по своему слоту, толпа набивается от цели наружу) и разная цена в
/// пройденном пути. Дефолт — [`SlotMatching::Greedy`], то есть сегодняшнее
/// поведение байт-в-байт.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SlotMatching {
    /// Пешка за пешкой: каждая берёт ближайший к цели свободный слот, а при
    /// равенстве — тот, что с её стороны ([`DestinationClaims::claim_slot`]).
    #[default]
    Greedy,
    /// Слот за слотом, от КРАЯ пачки к центру: сначала собираются `n` ближайших
    /// к цели свободных слотов (та же пачка, что раздал бы жадный), потом
    /// каждый слот, начиная с самого дальнего, достаётся ближайшей к нему
    /// пешке.
    ///
    /// Зачем от края. Цена промаха пропорциональна тому, насколько слот
    /// удалён от цели: путь пешки с обода радиуса `R` до слота в `ρ` от центра
    /// равен `R − ρ·cos(разность углов)`, то есть у центрального слота
    /// (`ρ = 0`) угол не значит ничего, а у крайнего решает всё. Жадный по
    /// пешкам раздаёт наоборот — сначала центр, а крайние слоты достаются тем,
    /// кто остался, то есть ровно там, где промах дороже всего, выбора уже нет.
    Batch,
}

/// Сторона слота в тайлах: `ceil(дистанция покоя / navtile_size())`, но не
/// меньше одного тайла.
pub fn slot_side(rest_distance: f32) -> i32 {
    (rest_distance / navtile_size()).ceil().max(1.0) as i32
}

/// Слот, которому принадлежит тайл. `div_euclid`, а не деление: тайлы у края
/// карты бывают отрицательными, а обычное деление на нуле «складывается»
/// и склеило бы два соседних слота в один.
pub fn slot_of(tile: IVec2, side: i32) -> IVec2 {
    IVec2::new(tile.x.div_euclid(side), tile.y.div_euclid(side))
}

/// Цель слота — центральный тайл блока. Только он, см. док модуля.
pub fn slot_target(slot: IVec2, side: i32) -> IVec2 {
    slot * side + IVec2::splat(side / 2)
}

/// Кто какой слот занял. Индекс, а не владелец: сущность без
/// [`DestinationClaim`] здесь появиться не может.
#[derive(Resource, Default)]
pub struct DestinationClaims {
    by_slot: HashMap<IVec2, Entity>,
    /// Сторона слота, на которой построен индекс. Ползунок радиуса и
    /// переключатель навтайла меняют её на лету, а ключи от прошлой решётки
    /// после этого не значат ничего.
    side: i32,
    /// Докуда искать свободный слот, м — снимок [`SlotSearch`] на прогон.
    search_meters: f32,
}

impl DestinationClaims {
    /// Свободен ли слот для этого претендента. Своя же заявка помехой себе не
    /// является — иначе повторный выбор той же цели уводил бы пешку в сторону.
    pub fn is_free(&self, slot: IVec2, claimant: Entity) -> bool {
        self.by_slot
            .get(&slot)
            .is_none_or(|owner| *owner == claimant)
    }

    /// Принять текущие настройки решётки: сторону слота и радиус поиска.
    ///
    /// Смена стороны выбрасывает индекс — старые ключи не пересчитываются:
    /// заявки восстановятся сами при следующем выборе цели, а неверная
    /// занятость держалась бы весь прогон. Радиус поиска ничего не
    /// перекеивает, он живёт только внутри одного поиска.
    pub fn sync(&mut self, side: i32, search_meters: f32) {
        if self.side != side {
            self.by_slot.clear();
            self.side = side;
        }
        self.search_meters = search_meters;
    }

    /// Занять слот под `desired` или ближайший свободный к нему; вернуть слот и
    /// его целевой тайл. `from` — где пешка стоит сейчас.
    ///
    /// Ближайший — по расстоянию ДО ЦЕЛИ: толпа обязана набиваться от цели
    /// наружу, иначе первые же пришедшие встанут по краю круга поиска и в
    /// середине останется дыра. Но среди одинаково близких к цели слот достаётся
    /// тому, кто ближе К ПЕШКЕ, то есть лежащему с её стороны. Без этого выбор
    /// внутри кольца произволен: пешка с одного края обода получала место у
    /// противоположного, шла через всю толпу и толкалась со встречными всю
    /// дорогу.
    ///
    /// `None` — если в пределах [`SlotSearch`] свободного слота с
    /// проходимой целью нет. Тогда цель остаётся общей и БЕЗ заявки: это ровно
    /// сегодняшнее поведение, и оно лучше, чем застопорить пешке выбор цели.
    ///
    /// Кольцо не знает о связности: свободный слот может оказаться за стеной,
    /// тогда поиск пути провалится и поведение выберет новую цель. Режим отказа
    /// тот же, что уже терпит `rescue_from_impassable`, и ограничен радиусом
    /// поиска.
    pub fn claim_slot(
        &mut self,
        claimant: Entity,
        previous: Option<IVec2>,
        desired: IVec2,
        from: Vec2,
        passable: impl Fn(IVec2) -> bool,
    ) -> Option<(IVec2, IVec2)> {
        // старую заявку снимаем ДО поиска: иначе пешка, выбравшая ту же
        // окрестность, обходила бы собственный прошлый слот
        if let Some(previous) = previous {
            self.release(previous, claimant);
        }
        let side = self.side;
        let radius = (self.search_meters / (side as f32 * navtile_size())).ceil() as i32;
        let slot = self.nearest_free_slot(
            claimant,
            slot_of(desired, side),
            radius.max(1),
            from,
            &passable,
        )?;
        self.by_slot.insert(slot, claimant);
        Some((slot, slot_target(slot, side)))
    }

    /// Раздать слоты ПАКЕТУ пешек, идущих в одну и ту же точку
    /// ([`SlotMatching::Batch`]). Ответ — по элементу на каждого члена пакета,
    /// в том же порядке; `None` там же, где его вернул бы [`Self::claim_slot`],
    /// то есть когда свободных слотов на всех не хватило.
    ///
    /// Раздаёт не пешкам, а СЛОТАМ, и начиная с самого дальнего от цели: см.
    /// [`SlotMatching::Batch`] о том, почему порядок именно такой.
    ///
    /// Цена — `O(радиус² + n·m)` на пакет против `n` кольцевых поисков у
    /// жадного. Оправдана она только когда в одну точку идёт целая пачка;
    /// одиночку вызывающий обязан отдать [`Self::claim_slot`] (это же и
    /// сохраняет дешёвым обычный случай игры, где у каждого блуждающего своя
    /// цель).
    pub fn claim_group(
        &mut self,
        desired: IVec2,
        members: &[(Entity, Option<IVec2>, Vec2)],
        passable: impl Fn(IVec2) -> bool,
        out: &mut Vec<Option<(IVec2, IVec2)>>,
    ) {
        out.clear();
        out.resize(members.len(), None);
        // старые заявки — ДО сбора свободных, всем пакетом: иначе члены пакета
        // обходили бы собственные прошлые слоты, и пачка уезжала бы от цели на
        // ровном месте (та же причина, что в `claim_slot`)
        for (entity, previous, _) in members {
            if let Some(previous) = previous {
                self.release(*previous, *entity);
            }
        }

        let side = self.side;
        let radius = ((self.search_meters / (side as f32 * navtile_size())).ceil() as i32).max(1);
        let anchor = slot_of(desired, side);
        let mut free: Vec<(i32, IVec2, IVec2)> = Vec::new();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let slot = anchor + IVec2::new(dx, dy);
                if self.by_slot.contains_key(&slot) {
                    continue;
                }
                let target = slot_target(slot, side);
                if !passable(target) {
                    continue;
                }
                free.push((dx * dx + dy * dy, slot, target));
            }
        }
        // пачка — те же `n` слотов, что раздал бы жадный: ближайшие к цели.
        // Хвост ключа по координатам, а не порядок обхода: на равном расстоянии
        // слотов бывает восемь, и выбор между ними обязан быть устойчивым
        free.sort_unstable_by_key(|(distance, slot, _)| (*distance, slot.x, slot.y));
        free.truncate(members.len());

        let mut taken = vec![false; members.len()];
        // от края пачки к центру, см. [`SlotMatching::Batch`]
        for (_, slot, target) in free.iter().rev() {
            let centre = tile_center(*target);
            let mut best: Option<(f32, usize)> = None;
            for (index, (_, _, from)) in members.iter().enumerate() {
                if taken[index] {
                    continue;
                }
                let cost = (centre - *from).length_squared();
                if best.is_none_or(|(previous, _)| cost < previous) {
                    best = Some((cost, index));
                }
            }
            let Some((_, index)) = best else {
                break;
            };
            taken[index] = true;
            out[index] = Some((*slot, *target));
            self.by_slot.insert(*slot, members[index].0);
        }
    }

    /// Свободный слот, ближайший к `desired`, а при равенстве — к `from`.
    ///
    /// Кольцевой поиск, как `navigation::nearest_tile_where`, но со вторым
    /// ключом, ради которого он и написан отдельно. Кольца чебышёвские, а
    /// первый ключ евклидов, поэтому кольцо, где нашёлся лучший кандидат, не
    /// последнее: тайл кольца `r` не ближе `r`, значит обход идёт, пока
    /// `r² ≤ лучшего` — на одно кольцо дальше, чем при поиске без второго
    /// ключа, чтобы кандидаты на том же расстоянии от цели не потерялись.
    fn nearest_free_slot(
        &self,
        claimant: Entity,
        desired: IVec2,
        radius: i32,
        from: Vec2,
        passable: &impl Fn(IVec2) -> bool,
    ) -> Option<IVec2> {
        let mut best: Option<(i32, f32, IVec2)> = None;
        for ring in 0..=radius {
            if let Some((to_target, ..)) = best
                && ring * ring > to_target
            {
                break;
            }
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    // только само кольцо: его внутренность просмотрена раньше
                    if ring > 0 && dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let slot = desired + IVec2::new(dx, dy);
                    if !self.is_free(slot, claimant) {
                        continue;
                    }
                    let target = slot_target(slot, self.side);
                    if !passable(target) {
                        continue;
                    }
                    let score = (
                        dx * dx + dy * dy,
                        (tile_center(target) - from).length_squared(),
                    );
                    if best.is_none_or(|(to_target, to_pawn, _)| score < (to_target, to_pawn)) {
                        best = Some((score.0, score.1, slot));
                    }
                }
            }
        }
        best.map(|(.., slot)| slot)
    }

    /// Снять заявку — только если слот числится за этой сущностью: после
    /// перестройки решётки ключ мог достаться кому-то другому.
    pub fn release(&mut self, slot: IVec2, claimant: Entity) {
        if self.by_slot.get(&slot) == Some(&claimant) {
            self.by_slot.remove(&slot);
        }
    }

    pub fn clear(&mut self) {
        self.by_slot.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_slot.len()
    }
}

/// Раздать слоты целому пакету пешек — единственная точка входа для обоих
/// вызывающих (система ниже и `drive_routes` стенда).
///
/// Пакет обязан прийти УЖЕ в том порядке, в котором его хотят обработать:
/// жадный режим раздаёт ровно по нему, пакетный — группирует по желаемой цели,
/// сохраняя порядок первого члена каждой группы. Ответ — по элементу на
/// каждого члена, в том же порядке; `None` значит «свободного слота нет, идти в
/// общую точку без заявки».
///
/// Группа из одного члена уходит в [`DestinationClaims::claim_slot`] при любом
/// режиме: пакетная раздача для неё вырождается в тот же ответ, а стоит
/// заметно дороже. В игре, где у каждого блуждающего своя цель, из одиночек
/// состоит почти весь пакет — то есть цена режима остаётся там, ради чего он и
/// заведён: в толпе, идущей в одну точку.
pub fn claim_batch(
    claims: &mut DestinationClaims,
    matching: SlotMatching,
    batch: &[(Entity, Option<IVec2>, IVec2, Vec2)],
    passable: impl Fn(IVec2) -> bool,
    out: &mut Vec<Option<(IVec2, IVec2)>>,
) {
    out.clear();
    out.resize(batch.len(), None);
    if matching == SlotMatching::Greedy {
        for (index, (entity, previous, desired, from)) in batch.iter().enumerate() {
            out[index] = claims.claim_slot(*entity, *previous, *desired, *from, &passable);
        }
        return;
    }

    // группы «одна и та же желаемая цель», в порядке первого члена
    let mut groups: Vec<(IVec2, Vec<usize>)> = Vec::new();
    let mut by_target: HashMap<IVec2, usize> = HashMap::new();
    for (index, (_, _, desired, _)) in batch.iter().enumerate() {
        match by_target.get(desired) {
            Some(group) => groups[*group].1.push(index),
            None => {
                by_target.insert(*desired, groups.len());
                groups.push((*desired, vec![index]));
            }
        }
    }

    let mut members = Vec::new();
    let mut answers = Vec::new();
    for (desired, indices) in &groups {
        if let [only] = indices[..] {
            let (entity, previous, _, from) = batch[only];
            out[only] = claims.claim_slot(entity, previous, *desired, from, &passable);
            continue;
        }
        members.clear();
        members.extend(indices.iter().map(|index| {
            let (entity, previous, _, from) = batch[*index];
            (entity, previous, from)
        }));
        claims.claim_group(*desired, &members, &passable, &mut answers);
        for (slot, index) in answers.iter().zip(indices) {
            out[*index] = *slot;
        }
    }
}

/// Развести свежие заявки на поиск пути по своим слотам.
///
/// Врезка одна и не в системы поведения: так слоты покрывают блуждание людей,
/// блуждание демонов и тестового ходока, не трогая ни `human/*`, ни `demon/*`.
///
/// Перезапись `movable.state` обязательна и обязана быть условной: фильтр
/// устаревших ответов в `apply_result` сверяет `end_tile` ответа с целью в
/// состоянии, и разъехавшись, они выбросили бы каждый найденный путь.
#[allow(clippy::too_many_arguments)]
pub fn assign_destination_slots(
    mut commands: Commands,
    navmesh: Res<ArcNavmesh>,
    style: Res<HumanStyle>,
    search: Res<SlotSearch>,
    lab: Res<SlotLab>,
    mut claims: ResMut<DestinationClaims>,
    mut fresh: Query<
        (
            Entity,
            Option<&crate::rng::PawnId>,
            Has<crate::human::Human>,
            &crate::movement::SimPosition,
            &mut Movable,
            &mut PathfindingRequest,
            Option<&DestinationClaim>,
        ),
        (
            Added<PathfindingRequest>,
            Without<crate::human::HumanFleeTag>,
            Without<crate::demon::DemonChaseTag>,
        ),
    >,
    mut order: Local<Vec<(f32, u8, u32, Entity)>>,
    mut batch: Local<Vec<(Entity, Option<IVec2>, IVec2, Vec2)>>,
    mut slots: Local<Vec<Option<(IVec2, IVec2)>>>,
) {
    claims.sync(slot_side(style.body_radius * 2.0) + lab.slack, search.0);

    // Пакет обрабатывается от ближних к своей цели к дальним: место у цели
    // должно достаться тому, кто рядом, а не тому, у кого меньше `PawnId`.
    // Иначе пешка с дальнего края забирает середину, а стоящая вплотную уезжает
    // наружу — лишний крюк обоим и встречный поток между ними.
    //
    // Хвост ключа — вид и номер пешки: сам по себе порядок обхода архетипов
    // ничего не гарантирует, а равные расстояния в толпе обычное дело. Тот же
    // ключ, что у детерминированного диспетчера.
    order.clear();
    order.extend(
        fresh
            .iter()
            .map(|(entity, pawn_id, is_human, position, _, request, _)| {
                (
                    (tile_center(request.end_tile) - position.0).length_squared(),
                    u8::from(is_human),
                    pawn_id.map_or(u32::MAX, |pawn_id| pawn_id.0),
                    entity,
                )
            }),
    );
    if order.is_empty() {
        return;
    }
    order.sort_unstable_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| (left.1, left.2).cmp(&(right.1, right.2)))
    });

    batch.clear();
    batch.extend(order.iter().filter_map(|(.., entity)| {
        let (_, _, _, position, _, request, claim) = fresh.get(*entity).ok()?;
        Some((
            *entity,
            claim.map(|claim| claim.0),
            request.end_tile,
            position.0,
        ))
    }));

    {
        let navmesh = navmesh.read();
        claim_batch(
            &mut claims,
            lab.matching,
            &batch,
            |tile| navmesh.is_passable(tile.x, tile.y),
            &mut slots,
        );
    }

    for ((entity, _, desired, _), slot) in batch.iter().zip(slots.iter()) {
        let Ok((_, _, _, _, mut movable, mut request, _)) = fresh.get_mut(*entity) else {
            continue;
        };
        match slot {
            Some((slot, target)) => {
                if target != desired {
                    request.end_tile = *target;
                    if movable.state == MovableState::Pathfinding(*desired) {
                        movable.state = MovableState::Pathfinding(*target);
                    }
                }
                commands.entity(*entity).insert(DestinationClaim(*slot));
            }
            // свободного слота нет — идём в общую точку, как раньше
            None => {
                commands.entity(*entity).remove::<DestinationClaim>();
            }
        }
    }
}

/// Вернуть на свой слот того, кого с него столкнули ([`SlotLab::regroup`]).
///
/// Кого это касается: пешка стои́т (`Idle`, то есть без
/// `MovableStateMovingTag`), её слот за ней числится, а сама она дальше
/// `regroup` метров от его целевого тайла. Заявка подаётся обычная, как из
/// поведения, — дальше работает тот же конвейер поиска пути.
///
/// **`Without<NeedsWanderTarget>` — не оптимизация, а граница ответственности.**
/// Тег висит ровно на тех, кому цель СЕЙЧАС выдаст поведение (`Idle` и
/// `PathfindingError`), и перебивать его возвратом нельзя: человек, дошедший до
/// цели, обязан пойти гулять дальше, а не топтаться на слоте. Возврат — для тех,
/// у кого источника целей нет: толпа, собранная у точки и оставленная стоять
/// (сцена `crowd_demo`, будущее «собраться у портала»), плюс всё, что придёт
/// на смену. В сегодняшней игре, где блуждают все, система не срабатывает ни
/// разу — и запрос у неё пуст, то есть стоит она ноль.
pub fn regroup_onto_slots(
    mut commands: Commands,
    lab: Res<SlotLab>,
    style: Res<HumanStyle>,
    mut settled: Query<
        (
            Entity,
            &mut Movable,
            &crate::movement::SimPosition,
            &DestinationClaim,
        ),
        (
            Without<crate::movement::MovableStateMovingTag>,
            Without<crate::movement::NeedsWanderTarget>,
        ),
    >,
) {
    if lab.regroup <= 0.0 {
        return;
    }
    let side = slot_side(style.body_radius * 2.0) + lab.slack;
    for (entity, mut movable, position, claim) in &mut settled {
        let home = slot_target(claim.0, side);
        if position.0.distance(tile_center(home)) <= lab.regroup {
            continue;
        }
        movable.to_pathfinding(
            entity,
            crate::grid::world_to_tile(position.0),
            home,
            &mut commands,
        );
    }
}

/// Снятие компонента — единственный путь освобождения индекса при сносе
/// сущности: деспавн поднимает `Remove` на каждый её компонент, и обсервер
/// успевает прочитать заявку, пока она ещё видна.
///
/// Покрывает побег за край карты, рестарт и смену города
/// (`DespawnOnExit(AppState::Playing)`), а также раздевание трупа.
pub fn on_destination_claim_removed(
    event: On<Remove, DestinationClaim>,
    mut claims: ResMut<DestinationClaims>,
    claimed: Query<&DestinationClaim>,
) {
    if let Ok(claim) = claimed.get(event.entity) {
        claims.release(claim.0, event.entity);
    }
}

/// Вход в мир начинается с пустого индекса.
pub fn reset_destination_claims(mut claims: ResMut<DestinationClaims>) {
    claims.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    fn anywhere(_: IVec2) -> bool {
        true
    }

    /// Индекс на решётке со стороной `side` тайлов.
    fn claims_with(side: i32) -> DestinationClaims {
        let mut claims = DestinationClaims::default();
        claims.sync(side, CLAIM_SEARCH_METERS);
        claims
    }

    /// Сторона слота округляется ВВЕРХ до целого числа тайлов: шаг решётки
    /// обязан быть не меньше дистанции покоя, иначе гарантии нет вовсе.
    #[test]
    fn the_slot_side_covers_the_rest_distance() {
        // навтайл по умолчанию 2 м: покой 1.8 умещается в один тайл
        assert_eq!(slot_side(1.8), 1);
        // а покой 2.4 (ползунок радиуса на максимуме) — уже нет
        assert_eq!(slot_side(2.4), 2);
        // вырожденный случай: слот не бывает меньше тайла
        assert_eq!(slot_side(0.1), 1);
    }

    /// Цели соседних слотов отстоят ровно на сторону слота — это и есть та
    /// гарантия, ради которой слот перестал быть одним тайлом.
    #[test]
    fn targets_of_neighbouring_slots_are_a_full_side_apart() {
        for side in 1..=3 {
            let here = slot_target(IVec2::new(4, 4), side);
            let right = slot_target(IVec2::new(5, 4), side);
            let up = slot_target(IVec2::new(4, 5), side);
            assert_eq!(right - here, IVec2::new(side, 0), "side {side}");
            assert_eq!(up - here, IVec2::new(0, side), "side {side}");
        }
    }

    /// Отрицательные тайлы не склеивают соседние слоты: у обычного деления
    /// −1 и 0 дают один и тот же ноль.
    #[test]
    fn slots_do_not_fold_around_the_origin() {
        assert_ne!(slot_of(IVec2::new(-1, 0), 2), slot_of(IVec2::new(0, 0), 2));
    }

    /// Свободный слот достаётся как есть — обычный случай, поиска нет.
    #[test]
    fn a_free_slot_is_claimed_as_is() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);

        assert_eq!(
            claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere),
            Some((desired, desired))
        );
    }

    /// Занятый слот уводит цель на соседний — иначе двое целятся в одну точку,
    /// и вечный упор между ними неизбежен.
    #[test]
    fn a_taken_slot_moves_the_claim_to_a_free_neighbour() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

        let (_, target) = claims
            .claim_slot(entity(2), None, desired, tile_center(desired), anywhere)
            .expect("сосед свободен");
        assert_ne!(target, desired);
        // соседний, а не любой: кольцевой поиск отдаёт ближайший
        assert_eq!((target - desired).abs().max_element(), 1);
    }

    /// На решётке со стороной 2 тайла цели двух пешек расходятся на два тайла,
    /// а не на один: ровно этого не умел слот-в-один-тайл при `NavtileBase::M1`.
    #[test]
    fn a_wide_slot_keeps_two_pawns_two_tiles_apart() {
        let mut claims = claims_with(2);
        let desired = IVec2::new(10, 10);
        let (_, first) = claims
            .claim_slot(entity(1), None, desired, tile_center(desired), anywhere)
            .expect("свободно");
        let (_, second) = claims
            .claim_slot(entity(2), None, desired, tile_center(desired), anywhere)
            .expect("сосед свободен");

        assert_eq!((first - second).abs().max_element(), 2);
    }

    /// Из одинаково близких к цели слотов пешке достаётся тот, что с ЕЁ
    /// стороны. Без этого выбор внутри кольца произволен, и подошедшие с разных
    /// краёв меняются местами: обе идут через занятую цель навстречу друг другу.
    #[test]
    fn a_pawn_takes_the_free_slot_on_its_own_side() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

        let west = tile_center(desired) - Vec2::new(40.0, 0.0);
        let east = tile_center(desired) + Vec2::new(40.0, 0.0);
        let (_, from_west) = claims
            .claim_slot(entity(2), None, desired, west, anywhere)
            .expect("сосед свободен");
        let (_, from_east) = claims
            .claim_slot(entity(3), None, desired, east, anywhere)
            .expect("сосед свободен");

        assert!(from_west.x < desired.x, "подошедший с запада: {from_west}");
        assert!(from_east.x > desired.x, "подошедший с востока: {from_east}");
    }

    /// Своя же заявка себе не помеха: пешка, выбравшая ту же цель второй раз,
    /// не должна уезжать в сторону от неё.
    #[test]
    fn an_own_claim_is_not_an_obstacle_to_itself() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        let (slot, _) = claims
            .claim_slot(entity(1), None, desired, tile_center(desired), anywhere)
            .expect("свободно");

        assert_eq!(
            claims.claim_slot(
                entity(1),
                Some(slot),
                desired,
                tile_center(desired),
                anywhere
            ),
            Some((slot, desired))
        );
        assert_eq!(claims.len(), 1);
    }

    /// Смена цели освобождает прошлый слот — иначе индекс за прогон зарастает
    /// заявками, которых никто не держит.
    #[test]
    fn re_claiming_frees_the_old_slot() {
        let mut claims = claims_with(1);
        let old = IVec2::new(10, 10);
        claims.claim_slot(entity(1), None, old, tile_center(old), anywhere);

        let far = IVec2::new(40, 40);
        claims.claim_slot(entity(1), Some(old), far, tile_center(far), anywhere);

        assert!(claims.is_free(old, entity(2)));
        assert_eq!(claims.len(), 1);
    }

    /// Свободного слота нет — цель остаётся общей и без заявки. Это сегодняшнее
    /// поведение: лучше, чем оставить пешку без цели вовсе.
    #[test]
    fn nothing_free_falls_back_to_the_shared_target() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);

        assert_eq!(
            claims.claim_slot(entity(1), None, desired, tile_center(desired), |_| false),
            None
        );
        assert_eq!(claims.len(), 0);
    }

    /// Пачка на общую цель: у каждого свой слот, ни один не повторился.
    #[test]
    fn a_batch_gives_every_member_its_own_slot() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        let members: Vec<_> = ring(8, 40.0, desired);
        let mut out = Vec::new();

        claims.claim_group(desired, &members, anywhere, &mut out);

        let slots: Vec<IVec2> = out.iter().map(|slot| slot.expect("место есть").0).collect();
        assert_eq!(slots.len(), 8);
        for (index, slot) in slots.iter().enumerate() {
            assert!(
                !slots[..index].contains(slot),
                "слот {slot} выдан дважды: {slots:?}"
            );
        }
        assert_eq!(claims.len(), 8);
    }

    /// То, ради чего пакетный режим и написан: суммарный путь пачки, идущей в
    /// одну точку, не больше, чем у жадного. Двенадцати пешек на ободе хватает,
    /// чтобы жадный начал раздавать крайние слоты тем, кто остался, — то есть
    /// не глядя на то, с какой стороны пешка подходит.
    #[test]
    fn a_batch_never_walks_further_than_the_greedy() {
        let desired = IVec2::new(10, 10);
        let members = ring(12, 40.0, desired);

        let cost = |matching| {
            let mut claims = claims_with(1);
            let batch: Vec<_> = members
                .iter()
                .map(|(entity, previous, from)| (*entity, *previous, desired, *from))
                .collect();
            let mut out = Vec::new();
            claim_batch(&mut claims, matching, &batch, anywhere, &mut out);
            out.iter()
                .zip(&members)
                .map(|(slot, (.., from))| {
                    let (_, target) = slot.expect("место есть");
                    (tile_center(target) - *from).length()
                })
                .sum::<f32>()
        };

        let greedy = cost(SlotMatching::Greedy);
        let batch = cost(SlotMatching::Batch);
        assert!(batch < greedy, "пакетный {batch} против жадного {greedy}");
    }

    /// Мест меньше, чем пешек, — хвост остаётся без слота и идёт в общую точку.
    /// Тот же режим отказа, что у одиночного поиска.
    #[test]
    fn a_batch_short_of_slots_leaves_the_tail_without_one() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        let members = ring(8, 40.0, desired);
        let mut out = Vec::new();

        // проходима ровно одна клетка — цель, и больше ничего
        claims.claim_group(desired, &members, |tile| tile == desired, &mut out);

        assert_eq!(out.iter().filter(|slot| slot.is_some()).count(), 1);
    }

    /// Пачка из одного — это обычный поиск: пакетный режим не должен менять
    /// ответ там, где выбирать не из чего (в игре из таких почти весь пакет).
    #[test]
    fn a_lone_member_of_a_batch_gets_the_greedy_answer() {
        let desired = IVec2::new(10, 10);
        let from = tile_center(desired) - Vec2::new(40.0, 0.0);
        let batch = [(entity(1), None, desired, from)];

        let answer = |matching| {
            let mut claims = claims_with(1);
            let mut out = Vec::new();
            claim_batch(&mut claims, matching, &batch, anywhere, &mut out);
            out[0]
        };

        assert_eq!(answer(SlotMatching::Batch), answer(SlotMatching::Greedy));
    }

    /// Пешки, равномерно расставленные по ободу вокруг цели.
    fn ring(count: u32, radius: f32, around: IVec2) -> Vec<(Entity, Option<IVec2>, Vec2)> {
        (0..count)
            .map(|index| {
                let angle = std::f32::consts::TAU * index as f32 / count as f32;
                (
                    entity(index + 1),
                    None,
                    tile_center(around) + Vec2::from_angle(angle) * radius,
                )
            })
            .collect()
    }

    /// Смена стороны слота выбрасывает индекс: ключи прошлой решётки не значат
    /// ничего, а неверная занятость держалась бы весь прогон.
    #[test]
    fn changing_the_slot_side_drops_the_index() {
        let mut claims = claims_with(1);
        let desired = IVec2::new(10, 10);
        claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

        claims.sync(2, CLAIM_SEARCH_METERS);

        assert_eq!(claims.len(), 0);
    }

    /// Снятие заявки чужим претендентом ничего не трогает.
    #[test]
    fn releasing_someone_elses_slot_is_a_no_op() {
        let mut claims = claims_with(1);
        let tile = IVec2::new(10, 10);
        let (slot, _) = claims
            .claim_slot(entity(1), None, tile, tile_center(tile), anywhere)
            .expect("свободно");

        claims.release(slot, entity(2));

        assert!(!claims.is_free(slot, entity(3)));
    }

    /// Деспавн освобождает слот — на нём держится вся уборка: побег за край,
    /// рестарт, смена города.
    #[test]
    fn despawn_releases_the_claim() {
        let mut app = App::new();
        app.init_resource::<DestinationClaims>()
            .add_observer(on_destination_claim_removed);

        let pawn = app.world_mut().spawn_empty().id();
        let tile = IVec2::new(10, 10);
        let slot = {
            let mut claims = app.world_mut().resource_mut::<DestinationClaims>();
            claims.sync(1, CLAIM_SEARCH_METERS);
            claims
                .claim_slot(pawn, None, tile, tile_center(tile), anywhere)
                .expect("свободно")
                .0
        };
        app.world_mut()
            .entity_mut(pawn)
            .insert(DestinationClaim(slot));

        app.world_mut().entity_mut(pawn).despawn();

        assert_eq!(app.world().resource::<DestinationClaims>().len(), 0);
    }
}
