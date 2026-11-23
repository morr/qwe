//! Дальний слой живых людей (Фаза 5 ×10 плана, `docs/population-x10-plan.md`):
//! дальняя пешка ([`FarTag`], спрайт скрыт) рисуется точкой в меше чанка —
//! тем же «точечным материалом» и той же решёткой, что трупные чанки
//! ([`super::bake`], шейдер `shaders/point.wesl`). Пол размера применяется
//! шейдером по юниформу зума — точка всегда [`HUMAN_MIN_PX`] логических
//! пикселей, поэтому смена зума не перестраивает ничего: у кромки кадра
//! спрайт и точка сходятся без шва (оба не мельче двух пикселей). Демоны
//! остаются спрайтами при всех зумах: их до 3500, ореол с блумом сохраняется.
//!
//! Источник правды точки — сам мир: позиция читается из `SimPosition`, цвет —
//! из `Attire` в момент перестройки чанка, копий в поле нет. Поэтому никаких
//! записей на срезном тике: ходок уходит на 0.35 м за срез — на зумах слоя это
//! доли пикселя, и точка, отставшая от пешки на несколько кадров, глазу не
//! видна. Перестройка — бюджетом за кадр: сначала грязные чанки (членство
//! менялось — точка обязана появиться или уехать сейчас), остаток — кольцу по
//! остальным чанкам, догоняющему позиции (свежеперестроенные грязные кольцо
//! пропускает — их позиции и так прочитаны живьём). Членство при жизни мира
//! заводит единственный обсервер — [`on_far_inserted`]; снять его никто не
//! обязан: перестройка сама выкидывает уехавших за клетку и не-дальней
//! (promote, труп, деспун снимают `FarTag` — запрос их не matchит), а
//! опустевший чанк удаляется целиком. Поле описывает живую сцену: на швах
//! жизненного цикла оно перечитывается по миру ([`on_world_started`]) или
//! уезжает вместе со сценой ([`clear_far_field`]).
//!
//! Детерминизму здесь нечего ломать: рендерная половина без RNG, членство
//! заводит только ярус (`assign_lod_tiers`, `SimPipeline::Live`) — под
//! тумблером детерминизма дальних не бывает, поле пусто. Без рендера материал
//! не инициализирован, и перестройка молча стоит — членство при этом копится
//! (таблица на чанк), реплей-двор это не поднимает.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use crate::camera::MAX_ZOOM;
use crate::human::{Attire, CorpseTag, FarTag, Human};
use crate::loading::{AppState, WorldStarted};
use crate::map::z::Z_FAR_CROWD;
use crate::movement::SimPosition;
use crate::settings::{HUMAN_MIN_PX, HUMAN_SIZE};

use super::Glyph;
use super::bake::{CorpseQuad, PointMaterialHandle, chunk_mesh};

/// Сторона клетки дальнего слоя, м. Вдвое мельче трупной (`CORPSE_CHUNK`
/// в `bake.rs`): дальних на карте до 200 тысяч против тысяч трупов, а
/// перестройка меша — стоимость на чанк, тем дешевле, чем мельче клетка.
/// Не семантические районы и не клетка трупов, хотя материал общий: у своего
/// слоя своя решётка.
const FAR_CHUNK: f32 = 250.0;

/// Сколько точек перестраивает [`rebuild_far_chunks`] за один кадр — бюджет
/// в **квадах, а не в чанках** нарочно: кольцо идёт по отсортированным ключам,
/// то есть по соседним клеткам карты, и в Париже соседние — это горячие
/// (плотный центр), чанк с десятком тысяч членов стоит миллисекунды чтения
/// (`SimPosition`/`Attire` каждого — случайный доступ по таблице сущностей).
/// Шесть соседних горячих чанков одним кадром дали бы пилу спайков каждый
/// круг; квадовый бюджет режет пилу: кадр стоит примерно одно и то же и при
/// лёгких клетках Тулы, и в центре Парижа.
const REBUILD_QUAD_BUDGET: usize = 12_000;

/// Полугабарит точки в метрах для честного AABB чанка: точка на экране всегда
/// [`HUMAN_MIN_PX`] логических пикселей, в метрах это `zoom · HUMAN_MIN_PX`,
/// а зум сверху ограничен [`MAX_ZOOM`]. Запас нужен, потому что у меша чанка
/// позиций-вершин нет (углы квадов — единичные, мир несёт атрибут): считать
/// AABB из `ATTRIBUTE_POSITION` значит отсечь чанк радиусом ~1.4 м у нуля —
/// ловушка, в которую угодили трупные чанки (там её закрыли
/// `NoFrustumCulling`; здесь вместо него честные границы, и камера отсекает
/// чанки за кадром по-настоящему).
const POINT_HALF_BOUND: f32 = MAX_ZOOM * HUMAN_MIN_PX / 2.0;

/// Метка меша одного чанка дальнего слоя. Спавнится перестройкой лениво —
/// чанк без дальних не существует; выборка `With<FarCrowdChunk>` — список
/// кусков слоя, её читают живые пробы (`brp count FarCrowdChunk`).
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FarCrowdChunk;

/// Клетка дальнего слоя: её члены и всё, что нужно, чтобы перерисовать меш.
#[derive(Default)]
struct FarChunkField {
    /// Дальние, живущие в клетке. `HashMap`, а не `Vec`: единственный писатель
    /// — обсервер перехода яруса, и повторный demote после promote обязан
    /// перезаписать членство, а не наращивать его (пешка у кромки кадра ходит
    /// ярусами туда-обратно — дублиры накопились бы за час).
    members: HashMap<Entity, ()>,
    /// Сущность и меш чанка, пока они существуют; первый же член их заводит.
    drawn: Option<(Entity, Handle<Mesh>)>,
    /// Членство приехало — меш надо пересобрать (вне очереди кольца).
    dirty: bool,
}

/// Хранилище дальнего слоя. Ключ — клетка [`FAR_CHUNK`]. Описывает живую
/// сцену: перечитывается по миру на [`WorldStarted`] ([`on_world_started`]),
/// уезжает вместе со сценой ([`clear_far_field`]).
#[derive(Resource, Default)]
pub struct FarCrowdField {
    chunks: HashMap<IVec2, FarChunkField>,
}

impl FarCrowdField {
    fn cell(&mut self, key: IVec2) -> &mut FarChunkField {
        self.chunks.entry(key).or_default()
    }
}

/// Клетка дальнего слоя точки. Деление с полом — и для отрицательных
/// координат клетка честная, а не «в минус первую».
fn chunk_key(at: Vec2) -> IVec2 {
    (at / FAR_CHUNK).floor().as_ivec2()
}

/// Пешка ушла в дальние: членство в клетке её позиции, клетка — грязная.
///
/// Единственный писатель членства при жизни мира (шов —
/// [`on_world_started`]). Срабатывает на flush команд яруса — точка
/// доезжает до меша перестройкой того же или следующего кадра. Обратного
/// обсервера нет: снятие яруса (promote, труп, деспун) пусть разбирает
/// перестройка — пометка «грязно» у кромки кадра стоила бы второй системы
/// ради точки, которая и так спрятана под спрайтом продвинутой пешки.
pub(crate) fn on_far_inserted(
    event: On<Insert<FarTag>>,
    mut field: ResMut<FarCrowdField>,
    pawns: Query<&SimPosition>,
) {
    let Ok(at) = pawns.get(event.entity) else {
        return;
    };
    let chunk = field.cell(chunk_key(at.0));
    chunk.members.insert(event.entity, ());
    chunk.dirty = true;
}

/// Новый прогон — слой перечитывается по живому миру, а не стирается.
///
/// `WorldStarted` приходит из двух мест, и мир в них разный:
/// - **рестарт** (`restart.rs::on_restart`) — люди прошлого прогона уже
///   деспавнуты, новых насос расселения заведёт после: живых дальних нет,
///   слой пуст — чанки прошлого прогона долой, хранилище дочиста;
/// - **вход в `Live`** — та же популяция, что стояла за прогрев: популяция
///   встаёт пачками, ярус демотит, слой заселяется ещё за экраном загрузки.
///   Стирать хранилище здесь нельзя: членство заводит только
///   `On<Insert<FarTag>>`, а вставка дальней пешки сработала до события —
///   стёртое некому вернуть, и пешка стояла со скрытым спрайтом без точки,
///   пока её не поднимут в кадр и не опустят снова (обзор №74: 10 238 пешек
///   на Туле).
///
/// Одно правило на оба пути: **члены — все живые дальние**, каждая живая
/// клетка — грязная. Меш клетки переживает перечитывание, если его сущность
/// жива; осиротевшие клетки (и мёртвые сущности при живой) чанк теряют.
pub(crate) fn on_world_started(
    _event: On<WorldStarted>,
    mut commands: Commands,
    mut field: ResMut<FarCrowdField>,
    drawn: Query<Entity, With<FarCrowdChunk>>,
    far: Query<(Entity, &SimPosition), (With<Human>, With<FarTag>, Without<CorpseTag>)>,
) {
    let mut fresh: HashMap<IVec2, FarChunkField> = HashMap::default();
    for (entity, at) in &far {
        let chunk = fresh.entry(chunk_key(at.0)).or_default();
        chunk.members.insert(entity, ());
        chunk.dirty = true;
    }
    for (key, old) in field.chunks.drain() {
        match fresh.get_mut(&key) {
            // живость сущности — страховка: поле, пережившее сцену, не должно
            // тащить мёртвый меш в живую клетку (смену города закрывает
            // `clear_far_field`, эта ветка — чтобы её пропуск не стоил точки)
            Some(chunk) => chunk.drawn = old.drawn.filter(|(entity, _)| drawn.contains(*entity)),
            None => {
                if let Some((entity, _)) = old.drawn
                    && drawn.contains(entity)
                {
                    commands.entity(entity).despawn();
                }
            }
        }
    }
    field.chunks = fresh;
}

/// Сцена уходит (смена города): сущности чанков снимает `DespawnOnExit`,
/// хранилище — эта система, тем же швом и у владельца (как
/// `navigation::clear_map_backends`). Переживи поле сцену — кольцо
/// перестройки в прогреве нового мира обходит клетки прошлого: все члены
/// мертвы, опустевшая клетка ставит despawn уже снятой сущности — 712
/// предупреждений на Тулу (обзор №74).
pub(crate) fn clear_far_field(mut field: ResMut<FarCrowdField>) {
    field.chunks.clear();
}

/// Ключи кольца догоняющего прохода: не-грязные клетки, **отсортированные** по
/// `(y, x)` — соседние клетки карты идут в соседних шагах кольца, и бюджет
/// кадра ложится на кусок карты, а не на порядок хеш-таблицы (см.
/// [`REBUILD_QUAD_BUDGET`]).
fn ring_keys(field: &FarCrowdField, fresh: &HashSet<IVec2>) -> Vec<IVec2> {
    let mut keys: Vec<IVec2> = field
        .chunks
        .iter()
        .filter(|(key, chunk)| !chunk.dirty && !fresh.contains(*key))
        .map(|(key, _)| *key)
        .collect();
    keys.sort_by_key(|key: &IVec2| (key.y, key.x));
    keys
}

/// Перестройка чанков дальнего слоя — бюджетом [`REBUILD_QUAD_BUDGET`] точек
/// за кадр: сначала грязные (членство менялось — точка обязана появиться или
/// уехать сейчас), остаток — кольцу по остальным чанкам (свежие грязные
/// пропускаются): позиции дальних
/// ходоков догоняют живьём, между перестройками точка отстаёт от пешки на
/// метры — на зумах слоя это доли пикселя. Членство разбирается по пути:
/// запрос не matchит не-дальней (promote, труп, деспун сняли `FarTag` — член
/// выкидывается), уехавший за клетку переезжает в свою; опустевший чанк
/// удаляется вместе со своей сущностью.
///
/// Только с рендером: хэндл точечного материала — гейт, как у пекаря. Под
/// тумблером детерминизма дальних не бывает, поле пусто — и на этом весь тик
/// системы.
pub(crate) fn rebuild_far_chunks(
    mut commands: Commands,
    mut field: ResMut<FarCrowdField>,
    mut meshes: ResMut<Assets<Mesh>>,
    handle: Res<PointMaterialHandle>,
    pawns: Query<(&SimPosition, &Attire), (With<Human>, With<FarTag>, Without<CorpseTag>)>,
    mut ring: Local<usize>,
) {
    let mut budget = REBUILD_QUAD_BUDGET;

    // грязные — вне очереди, в порядке обхода хранилища (порядок перестройки
    // на экран не влияет: точки внутри меша не сортированы и раньше)
    let dirty: Vec<IVec2> = field
        .chunks
        .iter()
        .filter(|(_, chunk)| chunk.dirty)
        .map(|(key, _)| *key)
        .collect();
    // свежеперестроенные кольцо пропускает: их позиции прочитаны живьём
    // только что, догонять нечего, а горячая клетка Парижа при каждом взмахе
    // членства перестраивалась бы дважды за кадр
    let fresh: HashSet<IVec2> = dirty.iter().copied().collect();
    for key in dirty {
        if budget == 0 {
            break;
        }
        let written = rebuild_chunk(&mut commands, &mut field, &mut meshes, &handle, &pawns, key);
        budget = budget.saturating_sub(written);
    }

    // кольцо: не-грязные чанки по отсортированным ключам (см. `ring_keys`),
    // курсор в `Local`; за `члены всего слоя / остаток бюджета` кадров все
    // дальние успевают
    if budget > 0 {
        let keys = ring_keys(&field, &fresh);
        let total = keys.len();
        if total == 0 {
            return;
        }
        let from = (*ring) % total;
        let mut spent = 0;
        for step in 0..total {
            if budget == 0 {
                break;
            }
            let key = keys[(from + step) % total];
            let written =
                rebuild_chunk(&mut commands, &mut field, &mut meshes, &handle, &pawns, key);
            budget = budget.saturating_sub(written);
            spent = step + 1;
        }
        *ring = (from + spent) % total;
    }
}

/// Перестроить один чанк: перечитать членов, выкинуть уехавших и не-дальней,
/// пересобрать меш. Опустевший чанк удаляется целиком (сущность и запись).
/// Возвращает число точек в пересобранном меше — бюджет кадра считается в
/// квадах (см. [`REBUILD_QUAD_BUDGET`]).
fn rebuild_chunk(
    commands: &mut Commands,
    field: &mut FarCrowdField,
    meshes: &mut Assets<Mesh>,
    handle: &PointMaterialHandle,
    pawns: &Query<(&SimPosition, &Attire), (With<Human>, With<FarTag>, Without<CorpseTag>)>,
    key: IVec2,
) -> usize {
    // весь блок с заимствованием чанка — в скоупе: после него полю снова
    // можно класть переезжающих членов
    let (quads, movers, min, max, drawn) = {
        let Some(chunk) = field.chunks.get_mut(&key) else {
            return 0;
        };
        chunk.dirty = false;
        let mut quads: Vec<CorpseQuad> = Vec::with_capacity(chunk.members.len());
        // уехавшие из клетки и не-дальней: перечисляются во время обхода, потом
        // выкидываются — сам `HashMap` на ходу не мутируется
        let mut evict: Vec<Entity> = Vec::new();
        let mut movers: Vec<(Entity, IVec2)> = Vec::new();
        let mut bounds = (Vec2::MAX, -Vec2::MAX);
        for &entity in chunk.members.keys() {
            let Ok((at, attire)) = pawns.get(entity) else {
                // promote, труп или деспун сняли `FarTag` — точка больше не нужна
                evict.push(entity);
                continue;
            };
            let to = chunk_key(at.0);
            if to != key {
                movers.push((entity, to));
                evict.push(entity);
                continue;
            }
            quads.push(CorpseQuad {
                centre: at.0,
                // квад квадратный, полувысота тела — [`HUMAN_SIZE`]/2; пол размера
                // применяется шейдером, тот же, что у спрайта
                half: HUMAN_SIZE / 2.0,
                floor_px: HUMAN_MIN_PX,
                tint: attire.0.to_linear().to_f32_array(),
                cell: Glyph::Disc.cell() as u32,
                flip: false,
                angle: 0.0,
            });
            bounds.0 = bounds.0.min(at.0);
            bounds.1 = bounds.1.max(at.0);
        }
        for entity in &evict {
            chunk.members.remove(entity);
        }
        let drawn = chunk.drawn.take();
        (quads, movers, bounds.0, bounds.1, drawn)
    };

    for (entity, to) in movers {
        let target = field.cell(to);
        target.members.insert(entity, ());
        target.dirty = true;
    }

    if quads.is_empty() {
        // чанк опустел: сущность меша долой, запись удалить. Сущность здесь
        // жива: поле не переживает сцену (`clear_far_field`), а мёртвый меш
        // `WorldStarted` уносит вместе с его клеткой
        if let Some((entity, _)) = drawn {
            commands.entity(entity).despawn();
        }
        field.chunks.remove(&key);
        return 0;
    }

    let mesh = chunk_mesh(&quads);
    let aabb = Aabb::from_min_max(
        (min - POINT_HALF_BOUND).extend(-1.0),
        (max + POINT_HALF_BOUND).extend(1.0),
    );
    match drawn {
        Some((entity, existing)) => {
            meshes
                .insert(existing.id(), mesh)
                .expect("хэндл меша чанка жив");
            // границы членов уехали — честный AABB переписывается вместе с мешем
            commands.entity(entity).insert(aabb);
            // `drawn` забирал скоуп-блок — вернуть на место: без возврата чанк
            // считался бы никогда не рисованным, каждая перестройка спавнила
            // бы вторую сущность, а опустев, не деспавнила бы ни одной
            field.cell(key).drawn = Some((entity, existing));
        }
        None => {
            let mesh = meshes.add(mesh);
            let entity = commands
                .spawn((
                    FarCrowdChunk,
                    Mesh2d(mesh.clone()),
                    MeshMaterial2d(handle.0.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_FAR_CROWD),
                    aabb,
                    // честный AABB вставлен руками (позиции-вершины — единичные
                    // углы у нуля, авто-границы из них отсекают чанк радиусом
                    // ~1.4 м, см. [`POINT_HALF_BOUND`]); маркер выключает
                    // `calculate_bounds_2d`, который иначе перезаписал бы
                    // границы при каждой замене меша (`AssetChanged`)
                    NoAutoAabb,
                    DespawnOnExit(AppState::Playing),
                    Name::new(format!("far_crowd_{}_{}", key.x, key.y)),
                ))
                .id();
            let chunk = field.cell(key);
            chunk.drawn = Some((entity, mesh));
        }
    }
    quads.len()
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::VertexAttributeValues;

    use super::*;
    use crate::human::HumanPlugin;
    use crate::human::Pace;
    use crate::rng::PawnId;

    /// Дальний пешка в тестовом мире: позиция и одежда по месту, ярус
    /// вставляется как у demote — обсервер заводит членство сам.
    fn far_pawn(app: &mut App, pawn_id: u32, at: Vec2) -> Entity {
        let pawn = app
            .world_mut()
            .spawn((
                Transform::from_translation(at.extend(0.0)),
                Human,
                Pace(0.0),
                PawnId(pawn_id),
                SimPosition(at),
                Attire(Color::hsl(200.0, 0.4, 0.5)),
            ))
            .id();
        app.world_mut().entity_mut(pawn).insert(FarTag);
        app.world_mut().flush();
        pawn
    }

    /// Поле дальнего слоя с мешами: хранилище, ассеты, хэндл-гейт, обсервер.
    fn field_app() -> App {
        let mut app = App::new();
        app.init_resource::<FarCrowdField>()
            .init_resource::<Assets<Mesh>>()
            .insert_resource(PointMaterialHandle(Handle::default()))
            .add_observer(on_far_inserted)
            .add_observer(on_world_started);
        app
    }

    fn rebuild(app: &mut App) {
        app.world_mut()
            .run_system_once(rebuild_far_chunks)
            .expect("перестройка");
        app.world_mut().flush();
    }

    fn chunk_count(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<Entity, With<FarCrowdChunk>>()
            .iter(app.world())
            .count()
    }

    /// Вершины квадов меша единственного чанка: источник правды точки — мир,
    /// хранилище квадов не держит, читается собранный меш по хэндлу чанка.
    fn quad_attrs(app: &App) -> Vec<[f32; 4]> {
        let field = app.world().resource::<FarCrowdField>();
        let Some((_, handle)) = field
            .chunks
            .values()
            .next()
            .expect("клетка жива")
            .drawn
            .as_ref()
        else {
            return Vec::new();
        };
        let handle = handle.clone();
        match app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&handle)
            .expect("меш чанка жив")
            .attribute(crate::silhouette::bake::ATTRIBUTE_QUAD)
        {
            Some(VertexAttributeValues::Float32x4(values)) => {
                // квад — четыре вершины с одинаковым атрибутом; точка квада —
                // запись первого угла
                values.chunks_exact(4).map(|quad| quad[0]).collect()
            }
            other => panic!("квады — Float32x4, а не {other:?}"),
        }
    }

    #[test]
    fn a_demoted_pawn_gets_a_dot_where_it_stands() {
        let mut app = field_app();
        let at = Vec2::new(100.0, 200.0);
        let _pawn = far_pawn(&mut app, 3, at);
        rebuild(&mut app);

        let field = app.world().resource::<FarCrowdField>();
        let chunk = field.chunks.values().next().expect("клетка жива");
        assert_eq!(chunk.members.len(), 1, "членство заводит обсервер");
        assert_eq!(chunk_key(at), IVec2::new(0, 0), "клетка сетки 250 м");
        assert_eq!(chunk_count(&mut app), 1, "меш чанка заспавнен");

        // точка стоит на позиции пешки, пол размера как у спрайта
        let quads = quad_attrs(&app);
        assert_eq!(quads.len(), 1, "одна точка");
        assert_eq!([quads[0][0], quads[0][1]], [at.x, at.y], "точка на позиции");
        assert_eq!(quads[0][2], HUMAN_SIZE / 2.0, "полувысота тела");
        assert_eq!(quads[0][3], HUMAN_MIN_PX, "пол размера как у спрайта");
    }

    #[test]
    fn a_promoted_pawn_loses_its_dot_and_the_empty_chunk_goes_away() {
        let mut app = field_app();
        let pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        rebuild(&mut app);
        assert_eq!(chunk_count(&mut app), 1);

        // promote: ярус снят — запрос перестройки пешку не matchит
        app.world_mut().entity_mut(pawn).remove::<FarTag>();
        app.world_mut().flush();
        rebuild(&mut app);

        assert!(
            app.world().resource::<FarCrowdField>().chunks.is_empty(),
            "опустевший чанк удалён из хранилища"
        );
        assert_eq!(chunk_count(&mut app), 0, "сущность меша ушла с чанком");
    }

    #[test]
    fn a_walker_who_crossed_the_border_moves_to_the_right_chunk() {
        let mut app = field_app();
        let at = Vec2::new(10.0, 10.0);
        let pawn = far_pawn(&mut app, 3, at);
        rebuild(&mut app);
        assert_eq!(chunk_key(at), IVec2::ZERO, "первая клетка — нулевая");

        // ходок перешёл границу клетки, пока его чанк не перестраивался
        let new_at = Vec2::new(300.0, 10.0);
        app.world_mut().get_mut::<SimPosition>(pawn).unwrap().0 = new_at;
        rebuild(&mut app);

        let new_key = chunk_key(new_at);
        assert_ne!(new_key, IVec2::ZERO, "ходок перешёл в соседнюю клетку");
        let field = app.world().resource::<FarCrowdField>();
        let target = field
            .chunks
            .get(&new_key)
            .expect("членство переехало в новую клетку");
        assert!(target.members.contains_key(&pawn), "член у новой клетки");
        assert!(
            field.chunks.get(&IVec2::ZERO).is_none(),
            "опустевшая старая клетка удалена"
        );
    }

    #[test]
    fn the_budget_holds_and_eviction_clears_empty_chunks() {
        let mut app = field_app();
        // пять дальних в разных клетках — пять грязных чанков
        let pawns: Vec<Entity> = (0..5)
            .map(|i| {
                let at = Vec2::new(i as f32 * FAR_CHUNK + 10.0, 10.0);
                far_pawn(&mut app, i as u32, at)
            })
            .collect();
        assert_eq!(app.world().resource::<FarCrowdField>().chunks.len(), 5);

        // первый прогон: бюджет покрывает все пять — грязные перестроены разом
        rebuild(&mut app);
        assert_eq!(chunk_count(&mut app), 5, "все чанки перестроены");

        // пешки ушли (promote/despun в игре): членство висит до перестройки,
        // и та же перестройка опустевшие чанки удаляет
        for pawn in &pawns {
            app.world_mut().despawn(*pawn);
        }
        app.world_mut().flush();
        rebuild(&mut app);
        assert!(
            app.world().resource::<FarCrowdField>().chunks.is_empty(),
            "опустевшие чанки убраны"
        );
        assert_eq!(chunk_count(&mut app), 0);
    }

    /// Кольцо догоняющего прохода идёт по отсортированным клеткам: бюджет
    /// кадра ложится на кусок карты (соседние клетки — соседние шаги кольца,
    /// см. `REBUILD_QUAD_BUDGET`), а не на порядок хеш-таблицы.
    #[test]
    fn the_ring_walks_sorted_cell_keys() {
        let mut app = field_app();
        // пять дальних в клетках не по порядку сортировки; перестройка делает
        // все клетки чистыми, кольцо обязано вернуть их по (y, x)
        for (i, (x, y)) in [(2, 0), (0, 1), (1, 1), (0, 0), (1, 0)]
            .into_iter()
            .enumerate()
        {
            let _ = far_pawn(
                &mut app,
                i as u32,
                Vec2::new(x as f32 * FAR_CHUNK + 10.0, y as f32 * FAR_CHUNK + 10.0),
            );
        }
        rebuild(&mut app);

        let field = app.world().resource::<FarCrowdField>();
        let keys = ring_keys(field, &HashSet::default());
        assert_eq!(keys.len(), 5, "все чистые клетки в кольце");
        assert!(
            keys.windows(2)
                .all(|pair| (pair[0].y, pair[0].x) < (pair[1].y, pair[1].x)),
            "кольцо по отсортированным ключам, а не {keys:?}"
        );
    }

    #[test]
    fn a_second_demote_overwrites_membership_instead_of_doubling_it() {
        let mut app = field_app();
        let pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        // повторный demote (после promote) — тот же член в ту же клетку
        app.world_mut().entity_mut(pawn).remove::<FarTag>();
        app.world_mut().flush();
        app.world_mut().entity_mut(pawn).insert(FarTag);
        app.world_mut().flush();
        rebuild(&mut app);

        let field = app.world().resource::<FarCrowdField>();
        let chunk = field.chunks.values().next().expect("клетка жива");
        assert_eq!(chunk.members.len(), 1, "членство перезаписано, не удвоено");
        assert_eq!(quad_attrs(&app).len(), 1, "точка одна, не две");
    }

    /// Рестарт: `on_restart` деспавнит людей **до** того, как триггерит
    /// `WorldStarted`, новых заводит после — живых дальних в момент события
    /// нет, слой пуст: и хранилище, и сущности чанков (рестарт не проходит
    /// `Loading`, `DespawnOnExit` до них не дотягивается — иначе вторая волна
    /// спавнит вторую сущность на клетку).
    #[test]
    fn world_started_clears_the_field_on_a_restart() {
        let mut app = field_app();
        let pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        rebuild(&mut app);
        assert_eq!(chunk_count(&mut app), 1);

        // как `on_restart`: сцена прошлого прогона — долой, затем событие
        app.world_mut().despawn(pawn);
        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();

        assert!(
            app.world().resource::<FarCrowdField>().chunks.is_empty(),
            "новый прогон — слой пуст"
        );
        assert_eq!(chunk_count(&mut app), 0, "сущности чанков деспавнены");
    }

    /// Вход в `Live` — та же популяция, что стояла за прогрев: событие
    /// обязано перечитать членство, а не стереть его. Стирало: вставка
    /// `FarTag` дальней пешки сработала до события, а членство заводит только
    /// она — пешка оставалась со скрытым спрайтом и без точки, пока её не
    /// поднимут в кадр и не опустят снова (обзор №74: 10 238 пешек на Туле).
    #[test]
    fn world_started_keeps_the_pawns_that_went_far_during_warmup() {
        let mut app = field_app();
        let _pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        rebuild(&mut app);
        assert_eq!(chunk_count(&mut app), 1, "слой заселился за прогрев");

        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();

        let field = app.world().resource::<FarCrowdField>();
        let chunk = field
            .chunks
            .values()
            .next()
            .expect("клетка пережила событие");
        assert_eq!(chunk.members.len(), 1, "членство перечитано по живому миру");
        assert!(chunk.dirty, "меш пересобирается: состав мог поменяться");
        assert!(
            chunk.drawn.is_some(),
            "меш живой клетки остался на месте — город не мигает на входе в Live"
        );
        assert_eq!(chunk_count(&mut app), 1, "чанк той же клетки не пересоздан");

        rebuild(&mut app);
        assert_eq!(quad_attrs(&app).len(), 1, "точка на месте");
    }

    /// Сущность меша может уйти раньше записи о ней — сцена сняла её
    /// (`DespawnOnExit`), а поле пережило. Перечитывание не тащит мёртвый
    /// меш в живую клетку: перестройка завела бы точки в несуществующей
    /// сущности, и клетка осталась бы без чанка навсегда.
    #[test]
    fn world_started_drops_the_mesh_entity_the_scene_already_took() {
        let mut app = field_app();
        let _pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        rebuild(&mut app);
        let chunk_entity = app
            .world_mut()
            .query_filtered::<Entity, With<FarCrowdChunk>>()
            .iter(app.world())
            .next()
            .expect("чанк построен");

        app.world_mut().despawn(chunk_entity);
        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();

        let field = app.world().resource::<FarCrowdField>();
        let chunk = field.chunks.values().next().expect("клетка жива");
        assert!(
            chunk.drawn.is_none(),
            "мёртвая сущность не уехала в живую клетку"
        );
        rebuild(&mut app);
        assert_eq!(chunk_count(&mut app), 1, "клетка завела новый меш");
        assert_eq!(quad_attrs(&app).len(), 1, "точка на месте");
    }

    /// Смена города: сущности чанков снимает `DespawnOnExit`, хранилище —
    /// тем же швом, у владельца. Переживи оно сцену — кольцо перестройки в
    /// прогреве нового мира обходит клетки прошлого, все члены мертвы, и на
    /// каждую ставит despawn уже снятой сущности (обзор №74: 712
    /// предупреждений на Тулу).
    #[test]
    fn leaving_playing_clears_the_field_with_the_scene() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, HumanPlugin))
            .init_resource::<Assets<Mesh>>()
            .insert_resource(PointMaterialHandle(Handle::default()));
        let _pawn = far_pawn(&mut app, 3, Vec2::new(100.0, 200.0));
        rebuild(&mut app);
        assert_eq!(
            app.world().resource::<FarCrowdField>().chunks.len(),
            1,
            "слой заселён"
        );

        // сцена ушла: `DespawnOnExit` снял сущности чанков, затем —
        // расписание выхода из `Playing`
        let chunks: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<FarCrowdChunk>>()
            .iter(app.world())
            .collect();
        for entity in chunks {
            app.world_mut().despawn(entity);
        }
        app.world_mut().run_schedule(OnExit(AppState::Playing));
        app.world_mut().flush();

        assert!(
            app.world().resource::<FarCrowdField>().chunks.is_empty(),
            "хранилище уехало вместе со сценой"
        );
    }
}
