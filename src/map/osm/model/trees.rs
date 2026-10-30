//! Деревья модели: сырьё посадки (ряды и ноды), собранный набор [`TreeSet`],
//! раскладка аллей и то, из чего `MapData::compose_trees` его собирает.

use bevy::prelude::*;

/// Аллея из OSM (`natural=tree_row`): осевая полилиния и то, что данные знают о
/// самой посадке. Деревья по ней расставляет `planting::plant_rows`.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub points: Vec<Vec2>,
    /// Шаг посадки из тегов, м (`spacing`, либо длина / (`count` − 1)).
    /// `None` — в данных шага нет, и он берётся из ползунка плотности.
    ///
    /// Теги эти в OSM редки и полустандартны, так что почти каждый ряд —
    /// `None`; ветка «шаг из данных» проверяется тестом, а не городом.
    pub spacing: Option<f32>,
    /// Радиус кроны из `diameter_crown`, м. `None` — разыгрывается, как в лесу.
    pub radius: Option<f32>,
}

/// Одиночное дерево из OSM (`node natural=tree`): позиция и радиус кроны из
/// `diameter_crown`. `None` — радиус разыгрывается, как в лесу. Сажает
/// `planting::plant_standalone`; ноды в лесу и у аллей там же и отсеиваются.
#[derive(Debug, Clone, Copy)]
pub struct TreeNode {
    pub pos: Vec2,
    pub radius: Option<f32>,
}

/// Посаженное дерево: центр, радиус кроны и плотность, на которой оно
/// появляется (см. [`TreeSet`]).
pub type PlantedTree = (Vec2, f32, f32);

/// Деревья карты — позиции и пороги появления **одним значением**.
///
/// Раньше это были два поля `MapData`: `trees: Vec<(Vec2, f32)>` и
/// `tree_appears_at: Vec<f32>`, обязанные быть «той же длины и того же
/// порядка». Инвариант держала проза, а оба поля были `pub` в типе, который
/// упомянут в трёх десятках файлов, — то есть жить ему было негде. Здесь его
/// держит тип: поля приватны, пополняется набор одним приватным `push`, и
/// разъехаться им нечем.
///
/// Здесь же и **правило префикса**: ползунок плотности показывает не фильтр, а
/// начало набора. Лес засаживается по потолку плотности, деревья отсортированы
/// по порогу появления, и шаг ползунка вверх обязан только добавлять деревья, а
/// не переставлять уже стоящие. Порог считается по номеру дерева внутри своего
/// массива (`(номер + 1) · TREE_AREA_PER_TREE / площадь`), поэтому каждый лес
/// отдаёт ровно свою долю, даже если засаживался до упора и не добрал
/// запрошенного (см. `planting.rs`).
#[derive(Default, Debug, PartialEq)]
pub struct TreeSet {
    /// Центр и радиус кроны.
    positions: Vec<(Vec2, f32)>,
    /// На какой плотности (`TreeStyle::density`) появляется каждое, по
    /// возрастанию.
    appears_at: Vec<f32>,
}

impl TreeSet {
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Весь набор: центр и радиус каждого дерева, по возрастанию порога.
    pub fn positions(&self) -> &[(Vec2, f32)] {
        &self.positions
    }

    /// Сколько деревьев видно при этой плотности — **префикс, а не фильтр**
    /// (см. док типа). Прореживание от этого монотонно: шаг ползунка вверх
    /// только добавляет деревья, уже стоящие не переезжают.
    ///
    /// Породе прореживание ортогонально: её решает поле хвои по координатам
    /// ствола, так что доля хвои в прореженном наборе та же, а дерево при
    /// движении ползунка породу не меняет.
    pub fn visible_count(&self, density: f32) -> usize {
        self.appears_at.partition_point(|&at| at <= density)
    }

    /// Видимый при этой плотности префикс набора.
    pub fn visible(&self, density: f32) -> &[(Vec2, f32)] {
        &self.positions[..self.visible_count(density)]
    }

    /// Порог появления дерева `index` — тестам.
    #[cfg(test)]
    pub fn appears_at(&self, index: usize) -> f32 {
        self.appears_at[index]
    }

    /// Все пороги подряд — тестам, которые проверяют их возрастание.
    #[cfg(test)]
    pub fn thresholds(&self) -> &[f32] {
        &self.appears_at
    }

    /// Набор из готовых посадок — **тестам**. В игре его собирает только
    /// [`MapData::compose_trees`](super::MapData::compose_trees), и это ровно
    /// то, ради чего поля приватны:
    /// сложить два массива разной длины больше негде.
    #[cfg(test)]
    pub fn of(trees: impl IntoIterator<Item = PlantedTree>) -> Self {
        let mut set = Self::default();
        for tree in trees {
            set.push(tree);
        }
        set
    }

    pub(super) fn clear(&mut self) {
        self.positions.clear();
        self.appears_at.clear();
    }

    pub(super) fn reserve(&mut self, additional: usize) {
        self.positions.reserve(additional);
        self.appears_at.reserve(additional);
    }

    /// Единственная дверь внутрь, видимая родителю: `MapData::compose_trees`
    /// пополняет набор, и это ровно то, ради чего поля приватны.
    pub(super) fn push(&mut self, (position, radius, at): PlantedTree) {
        self.positions.push((position, radius));
        self.appears_at.push(at);
    }
}

/// Что делать с деревом аллеи, попавшим на занятое место.
///
/// Живёт здесь, а не в `planting`, потому что по нему собирается
/// [`MapData::trees`](super::MapData::trees): политика — часть состояния модели, а не только аргумент
/// посадки. Переключается на лету из панели Tree rows (`TreeRowStyle::placement`),
/// поэтому оба варианта считаются на загрузке разом (см. `planting::plant_rows`).
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TreeRowPlacement {
    /// Позиция из OSM как есть: выбрасываются только деревья в домах и в воде.
    ///
    /// Именно так, потому что ширина дороги у нас **синтезирована**
    /// по классу highway (8–16 м) и о настоящих кромках ничего не знает.
    /// Аллея вдоль бульвара сплошь и рядом лежит внутри этой ширины, и полная
    /// проверка `blocked` стёрла бы ровно те ряды, ради которых всё и делалось.
    #[default]
    Keep,
    /// Занятое место — сдвиг вперёд по ряду до свободного; не нашлось на длине
    /// одного шага — дерева нет. Дороги и газоны при этом уважаются полностью.
    Slide,
}

impl TreeRowPlacement {
    pub const ALL: [Self; 2] = [Self::Keep, Self::Slide];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Slide => "slide",
        }
    }
}

/// Что решает, **где именно** встанут деревья аллеи: политика размещения и то,
/// слушаем ли мы шаг из тегов OSM. Обе оси меняют позиции, а не вид, поэтому
/// каждое сочетание раскладывается на загрузке и переключение в UI остаётся
/// пересборкой массива, а не пересадкой (см. [`RowTrees`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRowLayout {
    pub placement: TreeRowPlacement,
    /// `true` — шаг берётся из `spacing` / `count`, и ползунок плотности такой
    /// ряд не прореживает. `false` — теги игнорируются, ряд живёт по ползунку
    /// наравне с лесом.
    pub osm_spacing: bool,
}

impl Default for TreeRowLayout {
    fn default() -> Self {
        // по умолчанию данные важнее ползунка: если картограф проставил шаг,
        // он знает про эту аллею больше, чем наша формула
        Self {
            placement: TreeRowPlacement::default(),
            osm_spacing: true,
        }
    }
}

impl TreeRowLayout {
    pub const ALL: [Self; 4] = [
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: false,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: false,
        },
    ];
}

/// Аллеи, разложенные под каждое сочетание [`TreeRowLayout`]. Четыре варианта
/// вместо одного стоят копейки — рядов на карте сотни против десятков тысяч
/// лесных деревьев, — а взамен переключение любой из двух ручек не трогает
/// индексы близости, которые строятся по всем домам и дорогам карты.
#[derive(Debug, Default)]
pub struct RowTrees([Vec<PlantedTree>; 4]);

impl RowTrees {
    fn slot(layout: TreeRowLayout) -> usize {
        TreeRowLayout::ALL
            .iter()
            .position(|&known| known == layout)
            .expect("TreeRowLayout::ALL перечисляет все сочетания")
    }

    pub fn get(&self, layout: TreeRowLayout) -> &[PlantedTree] {
        &self.0[Self::slot(layout)]
    }

    pub fn set(&mut self, layout: TreeRowLayout, trees: Vec<PlantedTree>) {
        self.0[Self::slot(layout)] = trees;
    }
}

/// Что входит в собранный [`MapData::trees`](super::MapData::trees): раскладка аллей и какие
/// источники деревьев включены. Тумблеры панелей выключают источник целиком —
/// сборка та же, выключенное слагаемое пустое.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeCompose {
    pub layout: TreeRowLayout,
    /// Лесные массивы ([`MapData::wood_trees`](super::MapData::wood_trees)).
    pub woods: bool,
    /// Аллеи выбранной раскладки ([`MapData::row_trees`](super::MapData::row_trees)).
    pub rows: bool,
    /// Одиночные деревья из OSM-нод
    /// ([`MapData::standalone_trees`](super::MapData::standalone_trees)).
    pub standalone: bool,
}

impl Default for TreeCompose {
    fn default() -> Self {
        Self {
            layout: TreeRowLayout::default(),
            woods: true,
            rows: true,
            standalone: true,
        }
    }
}
