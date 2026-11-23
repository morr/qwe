//! Настраиваемые ресурсы: их запоминание между запусками и общее условие
//! «пользователь покрутил ручку».
//!
//! Ресурс считается настраиваемым, если его правит UI-панель, хоткей или
//! запись по BRP: город, дебаг-тумблеры, алгоритм поиска пути, размер
//! навтайла, стили карты, режим стартовой позиции камеры. Про такой ресурс
//! спрашивают ровно две вещи — «сохранить его выбор» и «его только что
//! покрутили, пора пересобирать», — и обе живут здесь.
//!
//! Поверх первопартийного `bevy::settings` (см. upstream-пример
//! `window/persisting_window_settings.rs`): сами ресурсы помечены
//! `#[derive(SettingsGroup)]` в своих модулях, а `SettingsPlugin` при сборке
//! `App` читает `settings.toml` из системной папки настроек и накладывает
//! значения на уже созданные ресурсы — то есть до любого расписания, так что
//! и UI-панели, и первый спавн мира стартуют с сохранённым выбором.
//!
//! Поэтому плагин регистрируется **последним**: `SettingsPlugin` сканирует
//! реестр типов на своей сборке, и `register_type` остальных плагинов должны
//! к этому моменту уже отработать.
//!
//! Главный чекаут читает и пишет системную папку настроек, ворктриа — свой
//! локальный файл (см. [`settings_app_name`]): параллельные инстансы игры не
//! должны перезаписывать настройки друг друга.
//!
//! Запись — на любое изменение отслеживаемого ресурса, откуда бы оно ни
//! пришло: клик по кнопке, хоткей, правка через BRP. Пишем синхронно, а не
//! `SaveSettingsDeferred`: кликов мало, а отложенная запись теряется, если
//! выйти из игры в ту же секунду.
//!
//! Сам файл пишет одна команда — [`SavePrefs`]: перед отдачей библиотеке она
//! подрезает то, что TOML не умеет хранить (целое без знака шире `i64`), а
//! `SaveSettingsSync` вне её не зовётся.
//!
//! Обратный ход — [`ResetSettings`], кнопка `reset` во вкладке Debug:
//! все группы настроек разом возвращаются к своим `Default`.

use bevy::ecs::change_detection::DetectChangesMut;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::system::Command;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::{
    GetTypeRegistration, PartialReflect, ReflectMut, TypeRegistration, std_traits::ReflectDefault,
};
use bevy::settings::{ReflectSettingsGroup, SaveSettingsSync, SettingsPlugin};
use std::any::TypeId;
use std::path::{Path, PathBuf};

/// Обратное доменное имя из URL репозитория — как просит документация
/// `SettingsPlugin`. Определяет папку: на macOS
/// `~/Library/Preferences/com.github.morr.qwe/settings.toml`.
const APP_NAME: &str = "com.github.morr.qwe";

/// Локальная папка настроек ворктриа, внутри чекаута. Целиком в gitignore.
const WORKTREE_SETTINGS_DIR: &str = ".settings";

pub struct PrefsPlugin;

impl Plugin for PrefsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SettingsPlugin::new(&settings_app_name(
            std::env::current_dir().ok().as_deref(),
        )));
    }
}

/// Имя «приложения» для `SettingsPlugin` с учётом ворктриа.
///
/// Ворктриа — параллельный инстанс игры, и один `settings.toml` на всех
/// означал бы, что выходя, инстансы перезаписывают выбор друг друга. Поэтому
/// у ворктриа файл персональный: [`WORKTREE_SETTINGS_DIR`] в корне чекаута,
/// при первом запуске один раз скопированный из стандартного
/// ([`seed_worktree_settings`]). Главный чекаут живёт по-старому — в
/// системной папке настроек.
///
/// Локальный путь подставляется в `SettingsPlugin::new` как «app name»:
/// `SettingsStore` строит каталог как `preferences_dir().join(app_name)`, а
/// `Path::join` с абсолютным путём подменяет базу целиком, — так чтение и
/// запись уходят в чекаут без всякой другой настройки. Ворктриа узнаётся по
/// `.git`-**файлу** (в главном чекауте `.git` — каталог); точка отсчёта —
/// текущий каталог процесса: `cargo run` стартует из корня чекаута.
fn settings_app_name(cwd: Option<&Path>) -> String {
    let Some(local_dir) = worktree_settings_dir(cwd) else {
        return APP_NAME.to_string();
    };
    seed_worktree_settings(&local_dir, &standard_settings_file());
    match local_dir.to_str() {
        Some(path) => path.to_string(),
        None => APP_NAME.to_string(), // неюникодный путь — назад к стандарту
    }
}

/// Локальная папка настроек, если запуск идёт из ворктриа; `None` для
/// главного чекаута и для запуска вне репозитория.
fn worktree_settings_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    let root = find_checkout_root(cwd)?;
    // ворктриа — только `.git`-файл; каталог `.git` — главный чекаут
    root.join(".git")
        .is_file()
        .then(|| root.join(WORKTREE_SETTINGS_DIR))
}

/// Корень чекаута: ближайший каталог с `.git` среди стартового и его предков.
fn find_checkout_root(cwd: Option<&Path>) -> Option<PathBuf> {
    let mut dir = cwd?.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// Стандартный файл настроек — тот, который читает главный чекаут.
fn standard_settings_file() -> PathBuf {
    let prefs = bevy::platform::dirs::preferences_dir().unwrap_or_else(|| {
        // без системной папки (нет HOME?) брать нечего — некорректный путь
        // просто не существует, и сидирование тихо пропустится
        PathBuf::from("/")
    });
    prefs.join(APP_NAME).join("settings.toml")
}

/// Первый запуск ворктриа: локальных настроек ещё нет — один раз взять
/// стандартные. Свои уже есть (или стандартных нет) — ничего не делать.
fn seed_worktree_settings(local_dir: &Path, standard_file: &Path) {
    let local_file = local_dir.join("settings.toml");
    if local_file.exists() || !standard_file.exists() {
        return;
    }
    match std::fs::create_dir_all(local_dir)
        .and_then(|()| std::fs::copy(standard_file, &local_file))
    {
        Ok(_) => info!("worktree settings seeded from {}", standard_file.display()),
        Err(e) => warn!(
            "could not seed worktree settings from {}: {e}",
            standard_file.display()
        ),
    }
}

/// Регистрация настраиваемого ресурса: полный вызов — [`PrefsExt::register_pref`],
/// его половинки — [`PrefsExt::track_pref`] и его же запомненный список
/// [`TrackedPrefs`].
///
/// Оба метода вызываются **у владельца ресурса** — список сохраняемого
/// собирается из тех же плагинов, что заводят сами ресурсы, а не повторяется
/// отдельным перечнем в этом модуле. Ручное зеркало здесь и стояло, и ровно
/// свой класс ошибок и давало: `RoadStyle` в нём не хватало с самого начала,
/// так что его правки сохранялись, только если в тот же кадр менялся
/// какой-нибудь другой отслеживаемый ресурс.
pub trait PrefsExt {
    /// Заводит настраиваемый ресурс и берёт на себя весь его вызов
    /// регистрации: `init_resource` + `register_type` + `track_pref` разом.
    ///
    /// Один вызов вместо трёх — не укорочение, а защита: у квартета по вызову
    /// на каждое из трёх действий, и забытый — не ошибка сборки, а тихий отказ.
    /// Забытый `register_type` — [`ResetSettings`] молча пропускает группу
    /// (ход по реестру типов не находит её) и `bevy_settings` не накладывает
    /// на неё файл; забытый `track_pref` — правки молча не пишутся в
    /// `settings.toml`. Парность вызовов дополнительно держит guard-тест
    /// (`tests/prefs.rs`).
    ///
    /// Вызов остаётся **в файле владельца** — семантика «регистрирует
    /// владелец» не меняется, меняется только форма.
    fn register_pref<T: FromWorld + Resource + GetTypeRegistration>(&mut self) -> &mut Self;

    /// Правки ресурса уезжают в `settings.toml` — запись на каждое его
    /// изменение.
    ///
    /// Сам по себе нужен редко: норма — [`PrefsExt::register_pref`], и отдельных
    /// вызовов у метода сейчас нет. Ресурс, которому полный вызов не подходит,
    /// чаще не отслеживается вовсе: `SavedCameraView` меняется каждый кадр
    /// протяжки камеры, и запись по изменению означала бы перезапись файла на
    /// кадр (у него свой дебаунс, `camera::track_camera_view`); солнце
    /// (`map::SunStyle`) и форма дорог (`map::RoadShape`) пишутся на каждом
    /// пройденном делении ползунка, поэтому в файл уезжают через оседание —
    /// отслеживается их копия на карте (`map::SunOnMap`, `map::RoadShapeOnMap`),
    /// которая меняется один раз, когда ползунок отпустили. Пишется при этом
    /// весь файл, так что группа `sun` уезжает в него со своим текущим
    /// значением.
    ///
    /// Сохранение решает `SaveSettingsSync::IfChanged` — сюда попадает *весь*
    /// файл, если хоть одна его группа поменялась, поэтому лишний вызов ничего
    /// не пишет и команды нескольких ресурсов в одном кадре не складываются в
    /// несколько записей.
    fn track_pref<T: Resource>(&mut self) -> &mut Self;
}

impl PrefsExt for App {
    fn register_pref<T: FromWorld + Resource + GetTypeRegistration>(&mut self) -> &mut Self {
        self.init_resource::<T>()
            .register_type::<T>()
            .track_pref::<T>()
    }

    fn track_pref<T: Resource>(&mut self) -> &mut Self {
        self.init_resource::<TrackedPrefs>();
        self.world_mut()
            .resource_mut::<TrackedPrefs>()
            .insert::<T>();
        self.add_systems(Update, save_prefs.run_if(resource_changed::<T>))
    }
}

/// Типы, у которых `track_pref` уже вызван.
///
/// Рантайму не нужен — записи хватает системы на каждом ресурсе, — заведён для
/// guard-теста (`tests/prefs.rs`): тот обходит реестр типов так же, как ходит
/// [`ResetSettings`], и сверяет с этим списком обе стороны — «группа
/// зарегистрирована, но не отслежена» и «отслежено, но группа не
/// зарегистрирована» — ловя тот же класс тихих отказов, от которого ушёл
/// [`PrefsExt::register_pref`].
#[derive(Resource, Default)]
pub struct TrackedPrefs {
    type_ids: HashSet<TypeId>,
}

impl TrackedPrefs {
    fn insert<T: Resource>(&mut self) {
        self.type_ids.insert(TypeId::of::<T>());
    }

    /// Отслеживается ли тип — для guard-теста.
    pub fn contains(&self, type_id: &TypeId) -> bool {
        self.type_ids.contains(type_id)
    }

    /// Все отслеженные типы — для guard-теста.
    pub fn iter(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.type_ids.iter().copied()
    }
}

fn save_prefs(mut commands: Commands) {
    commands.queue(SavePrefs);
}

/// Потолок целого в `settings.toml`: TOML хранит целые со знаком, `i64`.
/// Тот же потолок на входе поля seed — `rng::MAX_SEED`.
const TOML_INT_MAX: u64 = i64::MAX as u64;

/// Запись настроек в файл — единственная дверь к `SaveSettingsSync`.
///
/// `bevy_settings` сериализует каждую группу через `toml::Value::try_from(..)
/// .unwrap()`, и значение, которое TOML не умеет хранить, роняет приложение
/// прямо при записи. Причём не только при записи своей группы: файл пишется
/// целиком, едва изменилась любая его группа, так что одна такая правка
/// делала смертельным и сохранение вида камеры.
///
/// Непредставимое, что реально доходит до записи, одно — целое без знака
/// (`u64`, `usize`) выше `i64`. С панели оно не приходит: слайдер ограничен
/// шкалой, поле seed — потолком `rng::MAX_SEED`; приходит записью по BRP.
/// Поэтому команда сначала подрезает такие целые до потолка во всех группах
/// ([`clamp_file_values`]) и только потом отдаёт файл библиотеке.
///
/// Это не кламп на чтении у владельца (`DemonStyle::cap()`,
/// `HordeStyle::wave_size()`): тот защищает математику потребителя от
/// мусора в диапазоне, который файл несёт спокойно, а здесь защищён сам файл.
///
/// Каждое место, которое пишет файл, зовёт эту команду — система
/// [`save_prefs`], [`ResetSettings`], оба сохранения вида камеры в
/// `camera.rs`. Так обход групп стоит только на записи, которая и так
/// случилась, и ему не нужно ребро порядка к каждой записывающей системе.
pub struct SavePrefs;

impl Command for SavePrefs {
    type Out = ();

    fn apply(self, world: &mut World) {
        clamp_file_values(world);
        SaveSettingsSync::IfChanged.apply(world);
    }
}

/// Подрезает в группах настроек мира целые, которые файл не унесёт.
fn clamp_file_values(world: &mut World) {
    let mut clamped: Vec<&'static str> = Vec::new();
    visit_settings_groups(world, |registration, entity, world| {
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            return;
        };
        let Some(mut current) = reflect_component.reflect_mut(world.entity_mut(entity)) else {
            return;
        };
        // обход мимо отметки изменения: первый же `&mut` через `Mut` метит
        // группу изменённой, а по метке переписывается файл и пересобираются
        // кроны, дороги и здания. Отмечаем только то, что вправду подрезали
        if clamp_file_overflow(current.bypass_change_detection().as_partial_reflect_mut()) {
            current.set_changed();
            clamped.push(registration.type_info().type_path_table().short_path());
        }
    });
    if !clamped.is_empty() {
        warn!(
            "settings: integers above the TOML range clamped in {}",
            clamped.join(", ")
        );
    }
}

/// Подрезает в значении — рекурсивно, до листьев — целые без знака выше
/// [`TOML_INT_MAX`]. `true`, если подрезано хоть одно.
fn clamp_file_overflow(value: &mut dyn PartialReflect) -> bool {
    match value.reflect_mut() {
        ReflectMut::Struct(s) => visit_fields(s.field_len(), s, |s, i| s.field_at_mut(i)),
        ReflectMut::TupleStruct(s) => visit_fields(s.field_len(), s, |s, i| s.field_mut(i)),
        ReflectMut::Tuple(t) => visit_fields(t.field_len(), t, |t, i| t.field_mut(i)),
        // `Option` — тоже перечисление: `Some` несёт значение в поле варианта
        ReflectMut::Enum(e) => visit_fields(e.field_len(), e, |e, i| e.field_at_mut(i)),
        ReflectMut::List(l) => visit_fields(l.len(), l, |l, i| l.get_mut(i)),
        ReflectMut::Array(a) => visit_fields(a.len(), a, |a, i| a.get_mut(i)),
        ReflectMut::Opaque(leaf) => clamp_leaf(leaf),
        // словарей и множеств в группах настроек нет
        ReflectMut::Map(_) | ReflectMut::Set(_) => false,
    }
}

/// Обходит `len` дочерних значений контейнера; `true`, если подрезано хоть в
/// одном.
fn visit_fields<P: ?Sized>(
    len: usize,
    parent: &mut P,
    mut at: impl FnMut(&mut P, usize) -> Option<&mut dyn PartialReflect>,
) -> bool {
    let mut clamped = false;
    for i in 0..len {
        if let Some(field) = at(parent, i) {
            clamped |= clamp_file_overflow(field);
        }
    }
    clamped
}

fn clamp_leaf(leaf: &mut dyn PartialReflect) -> bool {
    if let Some(value) = leaf.try_downcast_mut::<u64>() {
        if *value > TOML_INT_MAX {
            *value = TOML_INT_MAX;
            return true;
        }
    }
    if let Some(value) = leaf.try_downcast_mut::<usize>() {
        if *value > TOML_INT_MAX as usize {
            *value = TOML_INT_MAX as usize;
            return true;
        }
    }
    false
}

/// Сброс всех настроек на умолчания — кнопка `reset` во вкладке Debug
/// (`ui/debug/mod.rs`). До неё вернуться к базовым значениям можно было только
/// удалив `settings.toml` руками.
///
/// **Списка ресурсов здесь нет и быть не должно.** Настройка узнаётся по той же
/// метке, по которой её знает сам `bevy_settings`, — `ReflectSettingsGroup` в
/// реестре типов, — а её умолчание берётся из `ReflectDefault`, который каждая
/// группа регистрирует своим `#[reflect(Resource, SettingsGroup, Default)]`.
/// Ручное зеркало списка тут повторило бы ошибку, от которой ушёл
/// [`PrefsExt::register_pref`]: новая настройка молча не сбрасывалась бы.
/// Ход по реестру — тот же, которым `bevy_settings` накладывает файл на мир
/// (`apply_settings_to_world`): тип → `ComponentId` → сущность ресурса.
///
/// Сбрасывается **всё**, включая мировые настройки: город, seed, детерминизм и
/// размер навтайла. Если они уведены от умолчания, мир после клика
/// перезагружается (`city::reload_world`) — это и значит «как при первом
/// запуске», а не «как сейчас, но с базовыми стилями».
///
/// Исключений нет ни одного, и `SavedCameraView` тоже сбрасывается. В режиме
/// `save` его в ту же секунду перезапишет `camera::track_camera_view` из
/// текущего положения камеры — и это верно: настройка здесь режим, а вид — то,
/// куда ты сейчас смотришь.
pub struct ResetSettings;

impl Command for ResetSettings {
    type Out = ();

    fn apply(self, world: &mut World) {
        let mut reset = 0usize;
        visit_settings_groups(world, |registration, entity, world| {
            let (Some(reflect_component), Some(reflect_default)) = (
                registration.data::<ReflectComponent>(),
                registration.data::<ReflectDefault>(),
            ) else {
                return;
            };

            let default = reflect_default.default();
            // сверка перед записью — не экономия, а корректность: `Mut` метит
            // ресурс изменённым по одному лишь взятию, и запись умолчания
            // поверх умолчания заказала бы пересборку крон, дорог, зданий и
            // полигонального меша на ровном месте
            let already_default = reflect_component
                .reflect(world.entity(entity))
                .and_then(|current| current.reflect_partial_eq(default.as_partial_reflect()))
                .unwrap_or(false);
            if already_default {
                return;
            }

            let Some(mut current) = reflect_component.reflect_mut(world.entity_mut(entity)) else {
                return;
            };
            // `apply` меняет и вариант перечисления — им же `bevy_settings`
            // кладёт варианты из TOML, так что City и прочие enum'ы проходят
            current.apply(default.as_partial_reflect());
            reset += 1;
        });

        info!("settings reset: {reset} group(s) back to defaults");

        if reset > 0 {
            // не полагаемся на то, что среди сброшенного оказался хоть один
            // `track_pref`-ресурс: `SavedCameraView` — группа настроек, но
            // отслеживается вручную. Запись сама сверяет тики файла, так
            // что лишний вызов ничего не пишет
            SavePrefs.apply(world);
        }
    }
}

/// Группы настроек, которые есть в этом мире, — обходом реестра типов.
///
/// Ход — тот же, которым `bevy_settings` накладывает файл на мир
/// (`apply_settings_to_world`): тип → `ComponentId` → сущность ресурса. Каждый
/// потребитель ([`ResetSettings`], запись файла) получает тип и сущность
/// группы и сам решает, что с ними делать.
fn visit_settings_groups(
    world: &mut World,
    mut visit: impl FnMut(&TypeRegistration, Entity, &mut World),
) {
    // клон Arc'а: гард реестра держится, пока мир правится, — так же, как в
    // `bevy_settings::apply_settings_to_world`
    let registry = world.resource::<AppTypeRegistry>().clone();
    let types = registry.read();

    for registration in types.iter() {
        if registration.data::<ReflectSettingsGroup>().is_none() {
            continue;
        }
        let Some(component_id) = world.components().get_id(registration.type_id()) else {
            continue;
        };
        let Some(entity) = world.resource_entities().get(component_id) else {
            continue;
        };
        visit(registration, entity, world);
    }
}

/// «Ручку покрутили»: ресурс изменён, и это не тот кадр, в котором он
/// появился.
///
/// Настройки накладываются на ресурсы при сборке `App`, поэтому в первом кадре
/// мира каждый настраиваемый ресурс числится и добавленным, и изменённым.
/// Пересборка по такому «изменению» в лучшем случае лишняя (мир только что
/// собран из этих же значений), в худшем — падает: деспавнить и пересобирать
/// ещё нечего.
pub fn retuned<T: Resource>(res: Option<Res<T>>) -> bool {
    res.is_some_and(|res| res.is_changed() && !res.is_added())
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::Command;

    use super::*;
    use crate::city::City;
    use crate::demon::HordeStyle;
    use crate::map::trees::{TreeShape, TreeStyle};
    use crate::rng::WorldSeed;

    /// Мир с парой настоящих групп настроек: структура с ручным `Default` и
    /// перечисление — у них разные пути в `PartialReflect::apply`.
    ///
    /// `SettingsPlugin` не нужен: сброс ходит по реестру типов, а запись на
    /// диск без реестра файлов сама превращается в предупреждение.
    fn world_with_settings() -> World {
        let mut world = World::new();
        let registry = AppTypeRegistry::default();
        {
            let mut types = registry.write();
            types.register::<TreeStyle>();
            types.register::<City>();
        }
        world.insert_resource(registry);
        world.init_resource::<TreeStyle>();
        world.init_resource::<City>();
        world
    }

    #[test]
    fn reset_returns_settings_to_defaults() {
        let mut world = world_with_settings();
        world.resource_mut::<TreeStyle>().shape = TreeShape::Conifer;
        *world.resource_mut::<City>() = City::ALL
            .iter()
            .copied()
            .find(|city| *city != City::default())
            .expect("нужен город, отличный от умолчания");

        ResetSettings.apply(&mut world);

        assert_eq!(
            world.resource::<TreeStyle>().shape,
            TreeStyle::default().shape
        );
        assert_eq!(*world.resource::<City>(), City::default());
    }

    /// Уже дефолтная группа не должна помечаться изменённой: по этой метке
    /// пересобираются кроны, дороги, здания и полигональный меш.
    #[test]
    fn reset_leaves_already_default_settings_untouched() {
        let mut world = world_with_settings();
        // увести одну группу, чтобы сброс вообще что-то сделал
        world.resource_mut::<TreeStyle>().shape = TreeShape::Conifer;
        world.clear_trackers();

        ResetSettings.apply(&mut world);

        assert!(world.resource_ref::<TreeStyle>().is_changed());
        assert!(!world.resource_ref::<City>().is_changed());
    }

    /// Временный каталог для тестов чекаута: уникальный, удаляется на выходе.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("qwe-prefs-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("создать временный каталог");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn checkout_root_walks_up_to_git() {
        let temp = TempDir::new("walk");
        let nested = temp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("вложенные каталоги");
        std::fs::create_dir(temp.path().join(".git")).expect("каталог .git");

        assert_eq!(
            find_checkout_root(Some(&nested)),
            Some(temp.path().to_path_buf())
        );
    }

    #[test]
    fn checkout_root_none_outside_repo() {
        let temp = TempDir::new("bare");
        assert_eq!(find_checkout_root(Some(temp.path())), None);
    }

    #[test]
    fn worktree_settings_dir_only_for_git_file() {
        let main = TempDir::new("main");
        std::fs::create_dir(main.path().join(".git")).expect("каталог .git");
        assert_eq!(worktree_settings_dir(Some(main.path())), None);

        let worktree = TempDir::new("worktree");
        std::fs::write(worktree.path().join(".git"), "gitdir: elsewhere\n").expect("файл .git");
        assert_eq!(
            worktree_settings_dir(Some(worktree.path())),
            Some(worktree.path().join(WORKTREE_SETTINGS_DIR))
        );
    }

    #[test]
    fn seed_copies_standard_once_and_needs_it() {
        let temp = TempDir::new("seed");
        let local_dir = temp.path().join(WORKTREE_SETTINGS_DIR);
        let standard = temp.path().join("standard.toml");
        std::fs::write(&standard, "[tree]\ndensity = 0.5\n").expect("стандартный файл");

        seed_worktree_settings(&local_dir, &standard);
        assert_eq!(
            std::fs::read_to_string(local_dir.join("settings.toml")).expect("локальный файл"),
            "[tree]\ndensity = 0.5\n"
        );

        // второй запуск: свои настройки есть, стандартные правками не мешаются
        std::fs::write(&standard, "[tree]\ndensity = 0.9\n").expect("правка стандарта");
        seed_worktree_settings(&local_dir, &standard);
        assert_eq!(
            std::fs::read_to_string(local_dir.join("settings.toml")).expect("локальный файл"),
            "[tree]\ndensity = 0.5\n"
        );

        // стандартного нет — сидировать нечего, локальный каталог не заводится
        let empty = TempDir::new("seed-empty");
        let missing = empty.path().join("missing.toml");
        seed_worktree_settings(&empty.path().join(WORKTREE_SETTINGS_DIR), &missing);
        assert!(!empty.path().join(WORKTREE_SETTINGS_DIR).exists());
    }

    /// Двор настоящей записи: `SettingsPlugin` с каталогом во временной
    /// папке — то же перенаправление абсолютным путём, каким ворктри держит
    /// свой `settings.toml`. Группы — обе формы, в которых бывает такое целое:
    /// `u64` в кортежной структуре и `usize` в обычной. `tag` задаёт каталог:
    /// тесты идут параллельно, и один тег на двоих — общий каталог, который
    /// первый же закончившийся тест удалит.
    fn save_yard(tag: &str) -> (App, TempDir) {
        let temp = TempDir::new(tag);
        let mut app = App::new();
        app.init_resource::<WorldSeed>()
            .register_type::<WorldSeed>()
            .init_resource::<HordeStyle>()
            .register_type::<HordeStyle>()
            .add_plugins(SettingsPlugin::new(temp.path().to_str().expect("путь в UTF-8")));
        // граница тика, которую в игре даёт кадр: плагин помнит тик своей
        // сборки как последнюю запись, и правка тем же тиком записи не ведёт
        app.world_mut().increment_change_tick();
        (app, temp)
    }

    fn plant_garbage(app: &mut App) {
        *app.world_mut().resource_mut::<WorldSeed>() = WorldSeed(u64::MAX);
        let mut horde = app.world_mut().resource_mut::<HordeStyle>();
        horde.wave_size = usize::MAX;
        horde.wave_len = 1 << 63;
    }

    /// Настоящая запись в файл. Без подрезки библиотека паникует на первом
    /// же таком значении (`toml::Value::try_from(..).unwrap()`).
    #[test]
    fn a_save_clamps_what_the_file_cannot_carry() {
        let (mut app, temp) = save_yard("clamp");
        plant_garbage(&mut app);

        SavePrefs.apply(app.world_mut());

        assert_eq!(app.world().resource::<WorldSeed>().0, TOML_INT_MAX);
        let horde = app.world().resource::<HordeStyle>();
        assert_eq!(horde.wave_size, TOML_INT_MAX as usize);
        assert_eq!(horde.wave_len, TOML_INT_MAX as usize);
        let file = std::fs::read_to_string(temp.path().join("settings.toml"))
            .expect("файл настроек записан");
        for line in [
            format!("seed = {TOML_INT_MAX}"),
            format!("wave_size = {}", TOML_INT_MAX as usize),
            format!("wave_len = {}", TOML_INT_MAX as usize),
        ] {
            assert!(file.contains(&line), "{line:?} нет в файле:\n{file}");
        }
    }

    /// Обход спускается во все формы: `Option`, список, массив, кортеж —
    /// полей такой формы в группах настроек сегодня нет, и проверить их
    /// больше нечем.
    #[test]
    fn clamp_reaches_every_shape() {
        let mut value = (
            u64::MAX,
            Some(usize::MAX),
            vec![0u64, u64::MAX],
            [usize::MAX; 2],
            i64::MIN,
            7u64,
        );

        assert!(clamp_file_overflow(&mut value));

        assert_eq!(
            value,
            (
                TOML_INT_MAX,
                Some(TOML_INT_MAX as usize),
                vec![0, TOML_INT_MAX],
                [TOML_INT_MAX as usize; 2],
                i64::MIN,
                7,
            )
        );
        // второй записи находить нечего
        assert!(!clamp_file_overflow(&mut value));
    }
}
