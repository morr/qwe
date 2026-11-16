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
//! Обратный ход — [`ResetSettings`], кнопка `reset` во вкладке Debug:
//! все группы настроек разом возвращаются к своим `Default`.

use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::system::Command;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::{GetTypeRegistration, std_traits::ReflectDefault};
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
    commands.queue(SaveSettingsSync::IfChanged);
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
        // клон Arc'а: гард реестра держится, пока мир правится, — так же, как в
        // `bevy_settings::apply_settings_to_world`
        let registry = world.resource::<AppTypeRegistry>().clone();
        let types = registry.read();

        let mut reset = 0usize;
        for registration in types.iter() {
            if registration.data::<ReflectSettingsGroup>().is_none() {
                continue;
            }
            let (Some(reflect_component), Some(reflect_default)) = (
                registration.data::<ReflectComponent>(),
                registration.data::<ReflectDefault>(),
            ) else {
                continue;
            };
            let Some(component_id) = world.components().get_id(registration.type_id()) else {
                continue;
            };
            let Some(entity) = world.resource_entities().get(component_id) else {
                continue;
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
                continue;
            }

            let entity_mut = world.entity_mut(entity);
            let Some(mut current) = reflect_component.reflect_mut(entity_mut) else {
                continue;
            };
            // `apply` меняет и вариант перечисления — им же `bevy_settings`
            // кладёт варианты из TOML, так что City и прочие enum'ы проходят
            current.apply(default.as_partial_reflect());
            reset += 1;
        }

        drop(types);
        info!("settings reset: {reset} group(s) back to defaults");

        if reset > 0 {
            // не полагаемся на то, что среди сброшенного оказался хоть один
            // `track_pref`-ресурс: `SavedCameraView` — группа настроек, но
            // отслеживается вручную. `IfChanged` сам сверяет тики файла, так
            // что лишний вызов ничего не пишет
            SaveSettingsSync::IfChanged.apply(world);
        }
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
    use crate::map::trees::{TreeShape, TreeStyle};

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
}
