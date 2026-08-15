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
//! Запись — на любое изменение отслеживаемого ресурса, откуда бы оно ни
//! пришло: клик по кнопке, хоткей, правка через BRP. Пишем синхронно, а не
//! `SaveSettingsDeferred`: кликов мало, а отложенная запись теряется, если
//! выйти из игры в ту же секунду.
//!
//! Обратный ход — [`ResetSettings`], кнопка `reset` в ряду дебаг-тумблеров:
//! все группы настроек разом возвращаются к своим `Default`.

use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::system::Command;
use bevy::prelude::*;
use bevy::reflect::std_traits::ReflectDefault;
use bevy::settings::{ReflectSettingsGroup, SaveSettingsSync, SettingsPlugin};

/// Обратное доменное имя из URL репозитория — как просит документация
/// `SettingsPlugin`. Определяет папку: на macOS
/// `~/Library/Preferences/com.github.morr.qwe/settings.toml`.
const APP_NAME: &str = "com.github.morr.qwe";

pub struct PrefsPlugin;

impl Plugin for PrefsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SettingsPlugin::new(APP_NAME));
    }
}

/// Регистрация настраиваемого ресурса: его правки уезжают в `settings.toml`.
///
/// Вызывается **у владельца ресурса**, рядом с его `init_resource` — список
/// сохраняемого собирается из тех же плагинов, что заводят сами ресурсы, а не
/// повторяется отдельным перечнем в этом модуле. Ручное зеркало здесь и стояло,
/// и ровно свой класс ошибок и давало: `RoadStyle` в нём не хватало с самого
/// начала, так что его правки сохранялись, только если в тот же кадр менялся
/// какой-нибудь другой отслеживаемый ресурс.
///
/// Сохранение решает `SaveSettingsSync::IfChanged` — сюда попадает *весь*
/// файл, если хоть одна его группа поменялась, поэтому лишний вызов ничего не
/// пишет и команды нескольких ресурсов в одном кадре не складываются в
/// несколько записей.
///
/// Регистрировать надо не всё сохраняемое: `SavedCameraView` меняется каждый
/// кадр протяжки камеры, и запись по изменению означала бы перезапись файла на
/// кадр. У него свой дебаунс — `camera::track_camera_view`.
pub trait TrackPrefExt {
    fn track_pref<T: Resource>(&mut self) -> &mut Self;
}

impl TrackPrefExt for App {
    fn track_pref<T: Resource>(&mut self) -> &mut Self {
        self.add_systems(Update, save_prefs.run_if(resource_changed::<T>))
    }
}

fn save_prefs(mut commands: Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
}

/// Сброс всех настроек на умолчания — кнопка `reset` в ряду дебаг-тумблеров
/// (`ui/debug/mod.rs`). До неё вернуться к базовым значениям можно было только
/// удалив `settings.toml` руками.
///
/// **Списка ресурсов здесь нет и быть не должно.** Настройка узнаётся по той же
/// метке, по которой её знает сам `bevy_settings`, — `ReflectSettingsGroup` в
/// реестре типов, — а её умолчание берётся из `ReflectDefault`, который каждая
/// группа регистрирует своим `#[reflect(Resource, SettingsGroup, Default)]`.
/// Ручное зеркало списка тут повторило бы ошибку, от которой ушёл
/// [`TrackPrefExt::track_pref`]: новая настройка молча не сбрасывалась бы.
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
}
