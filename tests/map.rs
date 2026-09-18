//! Тесты проводки слоёв карты — не того, что слой рисует, а того, как
//! `MapPlugin` ставит его системы в расписание.
//!
//! Пиннится здесь правило **«одно условие — одна регистрация»**, записанное в
//! доксах рядом со слоями (`roads::rebuilds_on`, на который ссылаются
//! остальные): две копии одной `rebuild_*` в одном расписании могут сработать
//! в одном кадре обе, и слой заспавнится дважды — деспавн второй копии идёт по
//! данным, снятым до применения команд первой. Это не теория: промзона приехала
//! с `rebuild_industry`, записанной в `Update` дважды.
//!
//! `rebuilds_on()` возвращает новый экземпляр условия на каждый вызов, так что
//! сложить условия через `or_else` вместо второй регистрации — договорённость,
//! а не свойство типа. До этого теста её стерегли только доксы.

use std::collections::BTreeMap;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use qwe::map::MapPlugin;

/// Сколько `rebuild_*` слоёв стоит в `Update` сегодня: дороги, дома, машины,
/// вагоны, заборы, промзона, рельсы, трамвай, кроны и полоса аллейных деревьев.
///
/// Нижняя граница, а не точное число: новый слой её поднимает, но ломать тест
/// незачем. Она здесь ради противоположного — чтобы фильтр, переставший что-либо
/// находить (переименовали модуль, `DebugName` остался без имён), не оставил
/// проверку зелёной на пустом множестве.
const LAYER_REBUILDS_IN_UPDATE: usize = 10;

/// Приложение с настоящим `MapPlugin`: проверяется именно его проводка, поэтому
/// плагин берётся целиком, а не по системам.
///
/// Мир не крутится и не будет — расписание читается сразу после `build`, до
/// того как `Schedule` разберёт граф в исполняемый вид. `AssetPlugin` нужен
/// ради `Material2dPlugin`: его `init_asset` требует `AssetServer`.
fn map_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::state::app::StatesPlugin,
        MapPlugin,
    ));
    app
}

/// Имя функции-системы без пути и без параметров типа:
/// `qwe::map::zoom::update_zoom_bucket<qwe::map::rail::RailLods>` → `update_zoom_bucket`.
fn short_name(name: &str) -> &str {
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit("::").next().unwrap_or(name)
}

#[test]
fn no_layer_rebuild_is_registered_twice_in_update() {
    let app = map_app();
    let update = app
        .get_schedule(Update)
        .expect("MapPlugin ставит системы в Update");

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, system, _) in update.graph().systems.iter() {
        let name = system.name().to_string();
        if name.starts_with("qwe::map::") && short_name(&name).starts_with("rebuild_") {
            *counts.entry(name).or_default() += 1;
        }
    }

    assert!(
        counts.len() >= LAYER_REBUILDS_IN_UPDATE,
        "фильтр нашёл {} слоёв вместо хотя бы {LAYER_REBUILDS_IN_UPDATE} — \
         проверка не на пустом ли она множестве? нашлось: {:?}",
        counts.len(),
        counts.keys().collect::<Vec<_>>(),
    );

    let twice: Vec<_> = counts.iter().filter(|&(_, &n)| n > 1).collect();
    assert!(
        twice.is_empty(),
        "в `Update` по две регистрации одной системы: {twice:?}. \
         Условия складываются через `or_else` в `rebuilds_on()` слоя, \
         а не разносятся по регистрациям — см. `roads::rebuilds_on`",
    );
}
