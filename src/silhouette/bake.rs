//! Запекание трупов: осевший труп уходит из спрайтов в меш чанка
//! ([`CorpseField`]), три сущности тройки деспавнятся, рост числа сущностей
//! останавливается.
//!
//! Свежий труп живёт как всегда: тело и два кровяных ребёнка спрайтами. Когда
//! лужа доросла, [`crate::human::spread_blood`] — тем же вызовом, каким
//! снимает `BloodSpread` — ставит тело в [`crate::human::BakeQueue`]: триггер
//! уже существует, сканирующих проходов нет. Пекарь ([`bake_queued_corpses`],
//! `Update`, только с рендером) пачкой за кадр переписывает тройку тремя
//! квадами в хранилище своего чанка ([`CORPSE_CHUNK`], фиксированная сетка, не
//! семантические районы), помечает чанк грязным и ставит тело в очередь
//! [`DespawnQueue`] — сам деспавн идёт в `FixedUpdate`, в хвосте
//! `HumanBehavior` рядом с `rise_souls` (правило CLAUDE.md «массовый деспавн
//! не из середины `Update`»).
//!
//! Квады рисует [`PointMaterial`] («точечный материал», шейдер
//! `shaders/point.wesl`): атлас тот же, что у свежих спрайтов, но квад
//! разворачивается в вершинном шейдере из «центр + полуразмер + пол», и пол
//! размера применяется по юниформу зума — запечённый чанк не перестраивается
//! при движении камеры. Порядок внутри меша — порядок треугольников (лужа и
//! брызги раньше тела), как сегодняшняя z-иерархия тройки; между трупами
//! сортировки и сейчас нет.
//!
//! Детерминизму здесь нечего ломать: рендерная половина без RNG, вид запечённой
//! тройки считают те же чистые функции, что рисуют свежую
//! (`corpse_pose` / `blood_look` / `corpse_tint`), а деспавн не трогает ничего,
//! что тик читает. Приложение без рендера (реплей, тесты) не запекает вовсе —
//! материал не инициализируется, трупы остаются сущностями, пины двора целы.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, PrimitiveTopology, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};

use crate::camera::Viewport;
use crate::human::{Attire, BakeQueue, CorpseTag, corpse_quads};
use crate::loading::{AppState, WorldStarted};
use crate::map::z::Z_CORPSE;
use crate::telemetry::Telemetry;

use super::{Glyph, Silhouettes};

const SHADER_PATH: &str = "shaders/point.wesl";

/// Сторона клетки трупной сетки, м. Меньше карты ([`MAP_CHUNK`] — километр)
/// нарочно: мельче клетка — дешевле перестройка меша, а на городском зуме
/// отсекать нечего и тридцать кусков на город — не цена. Не семантические
/// районы и не клетка слоёв: у трупов своя решётка и свой компромисс.
const CORPSE_CHUNK: f32 = 500.0;

/// Сколько трупов печёт [`bake_queued_corpses`] за один кадр. Очередь не горит
/// — труп никуда не спешит; всплеск после большой волны растягивается на
/// несколько кадров. Прецедент порций — `CrownStream::spawn_next` у крон.
const BAKE_BATCH: usize = 256;

/// Сколько грязных чанков перестраивает [`rebuild_dirty_chunks`] за кадр.
/// Грязных за кадр — единицы (пачка трупов ложится в один-два чанка), а
/// перестройка переписывает вершины чанка целиком — на нескольких тысячах
/// квадов это сотни микросекунд, и бюджет держит такие кадры по пальцам.
const REBUILD_CHUNK_BUDGET: usize = 4;

/// Сколько сущностей деспавнит [`despawn_baked_corpses`] за тик: пачка пекаря
/// разворачивается максимум в `BAKE_BATCH` тел по три сущности, потолок держит
/// тик при полной пачке.
const DESPAWN_BATCH: usize = 768;

/// Один запечённый квад: то, что раньше было сущностью со спрайтом. Половина
/// полей — вершины будущего меша, `angle`/`flip` — поворот и зеркало, всё это
/// разворачивает [`chunk_mesh`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CorpseQuad {
    /// Центр в метрах мира.
    pub centre: Vec2,
    /// Полуразмер; квад квадратный, и пол масштаба в шейдере считает от него.
    pub half: f32,
    /// Пол размера в логических пикселях — тот, что у спрайта нёс `Silhouette`.
    pub floor_px: f32,
    /// Тон, линейный RGBA — то, что у спрайта было `Sprite::color`.
    pub tint: [f32; 4],
    /// Ячейка атласа — [`Glyph::cell`]; UV-левая кромка в меш не пишется, её
    /// считает шейдер из номера ячейки.
    pub cell: u32,
    /// Зеркало глифа по горизонтали (`corpse_pose`), как `Sprite::flip_x`.
    pub flip: bool,
    /// Поворот квада, радианы против часовой: у тела — `heading` позы, у пятен
    /// — `heading + spin` (спрайт пятна — ребёнок тела и вращается вместе с
    /// ним). Вершины уходят в меш уже повёрнутыми.
    pub angle: f32,
}

/// Метка меша одного чанка трупов. Спавнится пекарем лениво — чанк без трупов
/// не существует; выборка `With<CorpseChunk>` — список кусков запечённого поля,
/// её читают живые пробы (`brp count CorpseChunk`).
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct CorpseChunk;

/// Клетка трупной сетки: её квад и всё, что нужно, чтобы перерисовать меш.
#[derive(Default)]
struct CorpseChunkField {
    /// Источник правды: меш чанка всегда перестраивается отсюда целиком.
    quads: Vec<CorpseQuad>,
    /// Сущность и меш чанка, пока они существуют; первый же квад их заводит.
    drawn: Option<(Entity, Handle<Mesh>)>,
    /// Квады приехали — меш надо пересобрать (бюджет перестроек — у пекаря).
    dirty: bool,
}

/// Хранилище запечённых трупов — источник правды трупных чанков. Ключ — клетка
/// [`CORPSE_CHUNK`]; сбрасывается тем же `WorldStarted`, что и счётчики прогона.
#[derive(Resource, Default)]
pub struct CorpseField {
    chunks: HashMap<IVec2, CorpseChunkField>,
}

impl CorpseField {
    fn cell(&mut self, key: IVec2) -> &mut CorpseChunkField {
        self.chunks.entry(key).or_default()
    }
}

/// Тела, запечённые в этом кадре, — очередь к деспавну. Пекарь пишет (он в
/// `Update`, откуда деспавнить нельзя), [`despawn_baked_corpses`] пьёт в
/// `FixedUpdate`, в хвосте `HumanBehavior` рядом с `rise_souls`.
#[derive(Resource, Default)]
pub struct DespawnQueue(pub Vec<Entity>);

/// Юниформ точечного материала — зеркало `PointUniform` в `point.wesl`:
/// порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct PointUniform {
    /// Метров на логический пиксель — зум камеры; пол размера применяется
    /// по нему той же формулой, что `Silhouette::drawn_size`.
    pub mpp: f32,
    /// Резерв раскладки: uv вершины считаются при сборке меша (`chunk_mesh`),
    /// паддинги держат юниформ кратным 16 байтам; шейдер их не читает.
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

/// Точечный материал — общий у трупных чанков и дальнего слоя живых (Фаза 5).
/// Атлас тот же, что у спрайтов; блендинг, как у них, — кромки фигур
/// сглажены, непрозрачный режим оставил бы ступеньки.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct PointMaterial {
    #[uniform(0)]
    pub params: PointUniform,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl Material2d for PointMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    /// Своя раскладка вершин: угол, цвет, квад и номер ячейки атласа — меш без
    /// них этим материалом не нарисовать, и это намеренно: собирать его надо
    /// через [`chunk_mesh`].
    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_QUAD.at_shader_location(2),
            ATTRIBUTE_CELL.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// Угол квада — атрибут ±1 по обеим осям; позицию несёт он, а не `Mesh`-евые
/// мировые координаты: разворачивает шейдер. Идентификатор — «высокий
/// случайный», как у атрибутов `map/meshing.rs`. Наружу модуля — ради тестов
/// дальнего слоя ([`super::far`]): его точки едут тем же атрибутом.
pub(crate) const ATTRIBUTE_QUAD: MeshVertexAttribute =
    MeshVertexAttribute::new("CorpseQuad", 1_884_661_903, VertexFormat::Float32x4);

/// UV вершины в атласе целиком: кромка ячейки, зеркало и вертикаль посчитаны
/// при сборке меша — угол квадада запечён в `position`, и выводить uv из
/// повёрнутого угла было бы ошибкой.
const ATTRIBUTE_CELL: MeshVertexAttribute =
    MeshVertexAttribute::new("CorpseCell", 1_504_397_251, VertexFormat::Float32x2);

/// Хэндл точечного материала. Настроечно его нет: без рендера (`Silhouettes`
/// не собран) материал не инициализируется, и его существование — гейт пекаря
/// и дальнего слоя (`far.rs` кладёт его на свои чанки).
#[derive(Resource)]
pub struct PointMaterialHandle(pub(crate) Handle<PointMaterial>);

/// Материал — на старте приложения, сразу после атласа: запечённым нужен тот
/// же `Handle<Image>`. Без рендера (`Silhouettes` пуст) ресурс не вставляется
/// вовсе — пекарь и перестройка чанков молча стоят.
pub fn init_point_material(
    silhouettes: Res<Silhouettes>,
    mut materials: ResMut<Assets<PointMaterial>>,
    mut commands: Commands,
) {
    let Some(atlas) = silhouettes.image() else {
        return;
    };
    let handle = materials.add(PointMaterial {
        params: PointUniform {
            // зум камеры приедет первым кадром `sync_point_uniform`; до тех
            // пор значение не на чем испортить кадр — запечённых ещё нет
            mpp: 1.0,
            pad0: 0.0,
            pad1: 0.0,
            pad2: 0.0,
        },
        atlas,
    });
    commands.insert_resource(PointMaterialHandle(handle));
}

/// Зум в юниформе материала — вслед за [`Viewport`], как
/// `resize_silhouettes_on_zoom` следит за камерой. Запечённые квад от зума не
/// хранят, поэтому одна правка юниформа перекрашивает все чанки разом.
pub(crate) fn sync_point_uniform(
    viewport: Option<Res<Viewport>>,
    handle: Res<PointMaterialHandle>,
    mut materials: ResMut<Assets<PointMaterial>>,
    mut last_view: Local<Option<Viewport>>,
) {
    let Some(view) = viewport else {
        return;
    };
    if *last_view == Some(*view) {
        return;
    }
    *last_view = Some(*view);
    if let Some(mut material) = materials.get_mut(&handle.0) {
        material.params.mpp = view.zoom;
    }
}

/// Клетка трупной сетки точки. Деление с полом — и для отрицательных координат
/// клетка считается честно, а не «в минус первую».
fn chunk_key(at: Vec2) -> IVec2 {
    (at / CORPSE_CHUNK).floor().as_ivec2()
}

/// Пекарь: очередь к запеканию — в квад чанков и в очередь деспавна.
///
/// Вид тройки не читается со спрайтов, а считается теми же чистыми функциями,
/// которые его нарисовали (`corpse_quads`): поза, кровь и тон живут в битах
/// `Entity` и `Attire`, а не в компонентах — потому запекание не зависит ни от
/// конвейера спрайтов, ни от атласа, а тестам не нужен рендер. Рост к этому
/// моменту закончился (`BakeQueue` ставит снятие `BloodSpread`), лужа — своего
/// полного размера.
pub(crate) fn bake_queued_corpses(
    mut queue: ResMut<BakeQueue>,
    mut field: ResMut<CorpseField>,
    mut despawn: ResMut<DespawnQueue>,
    mut telemetry: ResMut<Telemetry>,
    corpses: Query<(&GlobalTransform, Option<&Attire>), With<CorpseTag>>,
) {
    let batch = queue.0.len().min(BAKE_BATCH);
    for entity in queue.0.drain(0..batch) {
        // хвост из прошлого мира мог дожить до сброса — сущность не труп
        let Ok((at, attire)) = corpses.get(entity) else {
            continue;
        };
        let quads = corpse_quads(entity, at, attire);
        let chunk = field.cell(chunk_key(at.translation().xy()));
        chunk.quads.extend_from_slice(&quads);
        chunk.dirty = true;
        despawn.0.push(entity);
        telemetry.baked += 1;
    }
}

/// Перестройка грязных чанков — бюджетом [`REBUILD_CHUNK_BUDGET`] за кадр, как
/// «грязных за кадр — единицы» и обещано. Меш чанка всегда собирается из
/// хранилища целиком; хэндл у чанка один на жизнь, замена содержимого
/// (`insert`, а не `add`) не плодит ассетов.
pub(crate) fn rebuild_dirty_chunks(
    mut commands: Commands,
    mut field: ResMut<CorpseField>,
    mut meshes: ResMut<Assets<Mesh>>,
    handle: Res<PointMaterialHandle>,
) {
    let mut budget = REBUILD_CHUNK_BUDGET;
    for (key, chunk) in field.chunks.iter_mut() {
        if !chunk.dirty || budget == 0 {
            continue;
        }
        budget -= 1;
        chunk.dirty = false;
        let mesh = chunk_mesh(&chunk.quads);
        match &mut chunk.drawn {
            Some((_, existing)) => {
                meshes
                    .insert(existing.id(), mesh)
                    .expect("хэндл меша чанка жив");
            }
            slot @ None => {
                let mesh = meshes.add(mesh);
                let entity = commands
                    .spawn((
                        CorpseChunk,
                        Mesh2d(mesh.clone()),
                        MeshMaterial2d(handle.0.clone()),
                        Transform::from_xyz(0.0, 0.0, Z_CORPSE),
                        // Углы квадов в меш — единичные (±1), центр и угол
                        // несёт кастомный атрибут: по `ATTRIBUTE_POSITION`
                        // bevy считает AABB у origin радиусом ~1.4 м, и камера
                        // в любой точке карты, кроме угла у нуля, отсекала бы
                        // весь чанк (поймано живьём: широкое поле трупов
                        // видно, вблизи — пусто). Чанк городской сетки —
                        // честного AABB у меша нет, отсечение выключаем.
                        NoFrustumCulling,
                        DespawnOnExit(AppState::Playing),
                        Name::new(format!("corpse_chunk_{}_{}", key.x, key.y)),
                    ))
                    .id();
                *slot = Some((entity, mesh));
            }
        }
    }
}

/// Деспавн запечённых — единственный массовый деспавн corpses-сущностей, и он
/// в `FixedUpdate`, где ему место (CLAUDE.md, «Where a mass despawn may
/// happen»). Тела из очереди, уже не трупы (хвост прошлого мира), молча
/// пропускаются.
pub(crate) fn despawn_baked_corpses(
    mut commands: Commands,
    mut queue: ResMut<DespawnQueue>,
    corpses: Query<(), With<CorpseTag>>,
) {
    let batch = queue.0.len().min(DESPAWN_BATCH);
    for entity in queue.0.drain(0..batch) {
        if corpses.contains(entity) {
            commands.entity(entity).despawn();
        }
    }
}

/// Новый мир — запечённое поле пусто: чанки прошлого деспавнились компонентом
/// (`DespawnOnExit`), хранилище и очереди сбрасываются вслед за ними.
pub(crate) fn on_world_started(
    _event: On<WorldStarted>,
    mut field: ResMut<CorpseField>,
    mut despawn: ResMut<DespawnQueue>,
) {
    field.chunks.clear();
    despawn.0.clear();
}

/// Углы квада — против часовой; намотка треугольников держится на этом порядке.
const CORNERS: [Vec2; 4] = [
    Vec2::new(-1.0, -1.0),
    Vec2::new(1.0, -1.0),
    Vec2::new(1.0, 1.0),
    Vec2::new(-1.0, 1.0),
];

/// Меш чанка из его квадов: по четыре вершины и два треугольника на квад,
/// порядок квадов — порядок рисования (лужа и брызги раньше тела; между трупами
/// сортировки и в спрайтах не было). Поворот — в самих вершинах: угол уходит с
/// CPU, и шейдеру он не нужен. uv при этом считается из *неповёрнутого* угла —
/// интерполяция между повёрнутыми вершинами даёт тот же пиксель текстуры, что
/// повернутый спрайт. Зеркало глифа живёт тоже в uv: квад квадратный, зеркальная
/// геометрия его не меняет, и намотка от зеркала не зависит.
pub(crate) fn chunk_mesh(quads: &[CorpseQuad]) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let count = quads.len();
    let mut positions = Vec::with_capacity(count * 4);
    let mut colors = Vec::with_capacity(count * 4);
    let mut quad_attr = Vec::with_capacity(count * 4);
    let mut cells = Vec::with_capacity(count * 4);
    let mut indices = Vec::with_capacity(count * 6);
    for quad in quads {
        let base = positions.len() as u32;
        let (sin, cos) = quad.angle.sin_cos();
        let cell_u = quad.cell as f32 / Glyph::COUNT as f32;
        for corner in CORNERS {
            positions.push([
                corner.x * cos - corner.y * sin,
                corner.x * sin + corner.y * cos,
                0.0,
            ]);
            colors.push(quad.tint);
            quad_attr.push([quad.centre.x, quad.centre.y, quad.half, quad.floor_px]);
            let u = if quad.flip { -corner.x } else { corner.x };
            cells.push([
                cell_u + (u + 1.0) * 0.5 / Glyph::COUNT as f32,
                (1.0 - corner.y) * 0.5,
            ]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_attribute(ATTRIBUTE_QUAD, quad_attr);
    mesh.insert_attribute(ATTRIBUTE_CELL, cells);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn corpse_at(app: &mut App, at: Vec2, heading: f32) -> Entity {
        let transform =
            Transform::from_xyz(at.x, at.y, Z_CORPSE).with_rotation(Quat::from_rotation_z(heading));
        app.world_mut()
            .spawn((
                CorpseTag,
                Attire(Color::hsl(200.0, 0.4, 0.5)),
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
    }

    /// Пекарь в чистом мире: очередь, хранилище, очередь деспавна, телеметрия.
    fn bake_app() -> App {
        let mut app = App::new();
        app.init_resource::<BakeQueue>()
            .init_resource::<CorpseField>()
            .init_resource::<DespawnQueue>()
            .init_resource::<Telemetry>();
        app
    }

    /// Запечь `entity` и вернуть (число квадов, клетку хранилища).
    fn bake(app: &mut App, entity: Entity) -> (usize, IVec2) {
        app.world_mut().resource_mut::<BakeQueue>().0.push(entity);
        let mut baker = IntoSystem::into_system(bake_queued_corpses);
        baker.initialize(app.world_mut());
        baker.run((), app.world_mut()).unwrap();
        let field = app.world().resource::<CorpseField>();
        let (key, _) = field
            .chunks
            .iter()
            .next()
            .expect("запечённый труп завёл клетку");
        let quads = field
            .chunks
            .values()
            .map(|chunk| chunk.quads.len())
            .sum::<usize>();
        (quads, *key)
    }

    /// Тройка квадов в порядке рисования: брызги, лужа, тело — и счётчик
    /// телеметрии видит каждого запечённого.
    #[test]
    fn a_baked_corpse_lands_as_three_quads_in_draw_order() {
        let mut app = bake_app();
        let entity = corpse_at(&mut app, Vec2::new(100.0, 200.0), 0.7);
        let (quads, key) = bake(&mut app, entity);

        assert_eq!(quads, 3, "тройка: брызги, лужа, тело");
        assert_eq!(key, IVec2::new(0, 0), "клетка сетки 500 м");
        let pose = crate::human::corpse_pose(entity);
        let look = crate::human::blood_look(entity);
        let field = app.world().resource::<CorpseField>();
        let stored = &field.chunks[&key].quads;
        // порядок: брызги (шире всех), лужа, тело
        assert_eq!(stored[0].cell as usize, look.spatter.cell());
        assert_eq!(stored[1].cell as usize, look.pool.cell());
        assert_eq!(stored[2].cell as usize, pose.glyph.cell());
        assert!(stored[0].half > stored[1].half, "брызги шире лужи");
        assert_eq!(stored[2].flip, pose.flip, "зеркало тела приехало");
        assert_eq!(stored[2].centre, Vec2::new(100.0, 200.0), "центр тела");
        assert_eq!(stored[2].floor_px, crate::settings::HUMAN_MIN_PX);
        assert_eq!(
            app.world().resource::<Telemetry>().baked,
            1,
            "телметрия видит запечённого"
        );
        assert_eq!(
            app.world().resource::<DespawnQueue>().0,
            vec![entity],
            "тело встало в очередь деспавна"
        );
    }

    /// Кровь лежит под грудью в мире, а не в координатах клетки: поворот тела
    /// и зеркало позы доезжают до центров пятен.
    #[test]
    fn blood_quads_follow_the_chest_into_the_world() {
        let mut app = bake_app();
        // поза со смещённой от центра грудью — та же, что в тестах corpse.rs
        let heading = 1.234;
        let entity = corpse_at(&mut app, Vec2::new(-300.0, 40.0), heading);
        let pose = crate::human::corpse_pose(entity);
        bake(&mut app, entity);

        let field = app.world().resource::<CorpseField>();
        let stored = &field.chunks.values().next().unwrap().quads;
        // смещение груди в системе тела до поворота — `stain_transform`
        let anchor = pose.glyph.pool_anchor() * crate::human::CORPSE_SPAN / 2.0;
        let anchor = if pose.flip {
            Vec2::new(-anchor.x, anchor.y)
        } else {
            anchor
        };
        let turned = Quat::from_rotation_z(heading) * anchor.extend(0.0);
        let expected = Vec2::new(-300.0, 40.0) + turned.truncate();
        assert!(
            stored[1].centre.distance(expected) < 1e-4,
            "лужа у груди: {:?} вместо {expected:?}",
            stored[1].centre
        );
        assert!(
            stored[0].centre.distance(expected) < 1e-4,
            "брызги под той же грудью"
        );
    }

    /// Хвост очереди из прошлого мира — не труп: пекарь молча пропускает его и
    /// не портит ни хранилище, ни телеметрию.
    #[test]
    fn a_stale_queue_entry_is_skipped_without_a_trace() {
        let mut app = bake_app();
        let stale = Entity::from_raw_u32(42).unwrap();
        app.world_mut().resource_mut::<BakeQueue>().0.push(stale);
        let mut baker = IntoSystem::into_system(bake_queued_corpses);
        baker.initialize(app.world_mut());
        baker.run((), app.world_mut()).unwrap();

        assert!(app.world().resource::<CorpseField>().chunks.is_empty());
        assert_eq!(app.world().resource::<Telemetry>().baked, 0);
        assert!(
            app.world().resource::<DespawnQueue>().0.is_empty(),
            "не-труп и деспавнить нечего"
        );
    }

    /// Меш чанка: 4 вершины и 6 индексов на квад; угол поворачивает вершины,
    /// зеркало меняет только uv — намотка не зависит ни от того, ни от другого.
    #[test]
    fn chunk_mesh_turns_the_corners_and_keeps_the_winding() {
        let quad = CorpseQuad {
            centre: Vec2::ONE,
            half: 1.0,
            floor_px: 2.0,
            tint: [0.5; 4],
            cell: 3,
            flip: false,
            angle: 0.0,
        };
        let plain = chunk_mesh(&[quad]);
        assert_eq!(plain.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().len(), 4);
        let indices = |mesh: &Mesh| match mesh.indices() {
            Some(Indices::U32(indices)) => indices.clone(),
            _ => panic!("индексы U32"),
        };
        let winding = indices(&plain);
        assert_eq!(winding, vec![0, 1, 2, 2, 3, 0]);

        let positions = |mesh: &Mesh| match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            VertexAttributeValues::Float32x3(values) => values.clone(),
            other => panic!("позиции — Float32x3, а не {other:?}"),
        };

        // поворот на четверть оборота: угол против часовой гоняет углы по кругу
        // (sin_cos точного π/2 даёт ±0.9999999 — сверяемся с допуском)
        let quarter = positions(&chunk_mesh(&[CorpseQuad {
            angle: std::f32::consts::FRAC_PI_2,
            ..quad
        }]));
        let expected = [
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
            [-1.0, -1.0, 0.0],
        ];
        for (got, want) in quarter.iter().zip(expected.iter()) {
            assert!(
                Vec3::from(*got).distance(Vec3::from(*want)) < 1e-5,
                "углы повернулись на 90°: {quarter:?}, намотка та же"
            );
        }

        // зеркало — только в uv: u левого угла переехала на правую кромку ячейки
        let uvs = |mesh: &Mesh| match mesh.attribute(ATTRIBUTE_CELL).unwrap() {
            VertexAttributeValues::Float32x2(values) => values.clone(),
            other => panic!("uv — Float32x2, а не {other:?}"),
        };
        let cell_u = 3.0 / Glyph::COUNT as f32;
        let plain_uvs = uvs(&plain);
        assert_eq!(
            plain_uvs[0],
            [cell_u, 1.0],
            "левый нижний угол — левая кромка ячейки"
        );
        let flipped_uvs = uvs(&chunk_mesh(&[CorpseQuad { flip: true, ..quad }]));
        assert_eq!(
            flipped_uvs[0],
            [cell_u + 1.0 / Glyph::COUNT as f32, 1.0],
            "зеркало переставило левый угол на правую кромку"
        );
        assert_eq!(
            indices(&chunk_mesh(&[CorpseQuad { flip: true, ..quad }])),
            winding
        );
    }

    /// Перестройка: первый труп клетки спавнит её сущность, второй — только
    /// наращивает тот же меш. Хэндл один на жизнь чанка.
    #[test]
    fn a_chunk_spawns_once_and_grows_in_place() {
        let mut app = bake_app();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<PointMaterial>>()
            .insert_resource(PointMaterialHandle(Handle::default()));

        let first = corpse_at(&mut app, Vec2::ONE, 0.0);
        bake(&mut app, first);
        let mut rebuild = IntoSystem::into_system(rebuild_dirty_chunks);
        rebuild.initialize(app.world_mut());
        rebuild.run((), app.world_mut()).unwrap();
        app.world_mut().flush();

        let chunks: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<CorpseChunk>>()
            .iter(app.world())
            .collect();
        assert_eq!(chunks.len(), 1, "одна клетка — один кусок");
        let handle = app
            .world()
            .resource::<CorpseField>()
            .chunks
            .values()
            .next()
            .unwrap()
            .drawn
            .as_ref()
            .unwrap()
            .1
            .clone();
        let verts = |app: &App, handle: &Handle<Mesh>| {
            app.world()
                .resource::<Assets<Mesh>>()
                .get(handle)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .len()
        };
        assert_eq!(verts(&app, &handle), 12, "тройка квадов — 12 вершин");

        // второй труп той же клетки: сущность не спавнится, меш растёт
        let second = corpse_at(&mut app, Vec2::new(2.0, 2.0), 1.0);
        bake(&mut app, second);
        rebuild.run((), app.world_mut()).unwrap();
        app.world_mut().flush();
        let chunks: usize = app
            .world_mut()
            .query_filtered::<Entity, With<CorpseChunk>>()
            .iter(app.world())
            .len();
        assert_eq!(chunks, 1, "чанк один и тот же");
        assert_eq!(verts(&app, &handle), 24, "плюс тройка квадов");
    }

    /// Деспавн-слот: тело уходит, тройка целиком (дети — с родителем), хвост
    /// из прошлого мира молча пропускается.
    #[test]
    fn the_despawn_slot_takes_the_whole_triple_and_skips_stale_ids() {
        let mut app = bake_app();
        let (body, pool, spatter) = {
            let mut commands = app.world_mut().commands();
            let body = commands.spawn((CorpseTag, Name::new("body"))).id();
            let pool = commands.spawn(Name::new("pool")).id();
            let spatter = commands.spawn(Name::new("spatter")).id();
            commands.entity(body).add_child(pool);
            commands.entity(body).add_child(spatter);
            (body, pool, spatter)
        };
        // спавн-команды блока выше доезжают до мира здесь: без этого тело на
        // момент деспавна ещё не существует и скипается как хвост
        app.world_mut().flush();
        let stale = Entity::from_raw_u32(7).unwrap();
        app.world_mut()
            .resource_mut::<DespawnQueue>()
            .0
            .extend([stale, body]);
        let mut slot = IntoSystem::into_system(despawn_baked_corpses);
        slot.initialize(app.world_mut());
        slot.run((), app.world_mut()).unwrap();
        app.world_mut().flush();

        assert!(app.world().get_entity(pool).is_err(), "лужа ушла с телом");
        assert!(
            app.world().get_entity(spatter).is_err(),
            "брызги ушли с телом"
        );
        assert!(
            app.world().resource::<DespawnQueue>().0.is_empty(),
            "хвост дожат"
        );
    }
}
