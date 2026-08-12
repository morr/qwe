//! Какой бэкенд активен **сейчас** — одним значением.
//!
//! Вопрос «по чему сейчас ходят пешки» задавался семнадцатью местами в восьми
//! файлах, и — что хуже — в четырёх разных смыслах: «меш включён?» (гейт
//! расталкивания, секции панели), «включён и готов?» (сбор снимка поиска),
//! «выбранный бэкенд ещё строится?» (экран загрузки), «иерархию надо
//! запускать?» (условие постройки northstar). Каждый смысл собирался из своей
//! комбинации четырёх ресурсов, и совпадать они не обязаны были.
//!
//! Дороже всего обходилась одна склейка: `Pathfinder::polymesh_build`
//! возвращает `Option`, в котором «меш выключен» и «меш ещё строится» — один и
//! тот же `None`. Потребителю, которому нужно их различать, приходилось
//! собирать вопрос заново из сырых ресурсов; `examples/demos/crowd_demo.rs`
//! делает это с комментарием, прямо признающим, что переспрашивает то, на что
//! метод уже отвечает.
//!
//! Здесь состояние названо целиком. Таблица замкнута: пять вариантов
//! отвечают на все четыре вопроса, и ни один потребитель больше не обращается
//! к [`PolymeshDebug`](super::PolymeshDebug),
//! [`PathfindingAlgorithm`](super::PathfindingAlgorithm),
//! [`NorthstarGrid`](super::NorthstarGrid) или [`PolyNavmesh`](super::PolyNavmesh)
//! ради ветвления.
//!
//! **Значение, а не ресурс.** Каждый потребитель снимает его сам, в свой
//! момент: гейт загрузки и гейт расталкивания обязаны видеть живое положение
//! дел даже в детерминированном прогоне, где снимок поиска
//! ([`Backend`](super::Backend)) намеренно заморожен на весь прогон.

use std::sync::Arc;

use super::polymesh::PolymeshBuild;

/// Состояние сеточного бэкенда — того, что работает, пока полигональный меш
/// выключен.
#[derive(Clone)]
pub enum GridMode {
    /// Выбран плоский алгоритм; иерархия не нужна и не строится.
    Flat,
    /// Иерархия нужна выбранному алгоритму, но её ещё нет. Пока её нет,
    /// запросы обслуживает плоский A* — тот же приём, которым сетка
    /// обслуживает меш на время его постройки.
    HierarchyPending {
        /// Постройку ещё не запускали. Ровно это спрашивает условие
        /// `start_northstar_build`: двенадцать секунд всех ядер стоит запускать
        /// один раз и только под выбранный бэкенд.
        wanted: bool,
    },
    /// Иерархия построена и обслуживает поиск.
    Hierarchy(Arc<bevy_northstar::prelude::OrdinalGrid>),
}

/// Состояние полигонального бэкенда — того, что перекрывает сеточный, когда
/// включён.
#[derive(Clone)]
pub enum MeshMode {
    /// Включён, но ещё не построен (0.3–20 с). Запросы всё это время
    /// обслуживает сетка.
    Pending,
    /// Построен и обслуживает поиск.
    Ready(Arc<PolymeshBuild>),
}

/// По чему пешки ходят прямо сейчас.
#[derive(Clone)]
pub enum NavMode {
    Grid(GridMode),
    Mesh(MeshMode),
}

// Печатается только имя состояния: содержимое — построенная иерархия и
// полигональный меш, — `Debug` не имеет и в логе не нужно. Ручной impl, а не
// `derive`, именно поэтому.
impl std::fmt::Debug for NavMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Grid(GridMode::Flat) => "Grid(Flat)",
            Self::Grid(GridMode::HierarchyPending { wanted: true }) => "Grid(HierarchyWanted)",
            Self::Grid(GridMode::HierarchyPending { wanted: false }) => "Grid(HierarchyBuilding)",
            Self::Grid(GridMode::Hierarchy(_)) => "Grid(Hierarchy)",
            Self::Mesh(MeshMode::Pending) => "Mesh(Pending)",
            Self::Mesh(MeshMode::Ready(_)) => "Mesh(Ready)",
        };
        formatter.write_str(name)
    }
}

impl NavMode {
    /// Пути — метрические полилинии, а не центры тайлов.
    ///
    /// Отвечает **включённость**, а не готовность: пока меш строится, запросы
    /// обслуживает сетка, но мигать расталкиванием на этом переходе хуже, чем
    /// доработать полсекунды по-старому. Поэтому `Mesh(Pending)` — уже
    /// «непрерывно».
    pub fn is_continuous(&self) -> bool {
        matches!(self, Self::Mesh(_))
    }

    /// Ждать ли выбранный бэкенд. Спрашивается только про **выбранный**:
    /// иерархия northstar не нужна ни включённому мешу, ни плоскому A*, и
    /// ждать её в этих случаях значило бы держать экран загрузки зря.
    pub fn is_building(&self) -> bool {
        matches!(
            self,
            Self::Mesh(MeshMode::Pending) | Self::Grid(GridMode::HierarchyPending { .. })
        )
    }

    /// Запускать ли постройку иерархии сейчас.
    pub fn northstar_wanted(&self) -> bool {
        matches!(
            self,
            Self::Grid(GridMode::HierarchyPending { wanted: true })
        )
    }

    /// Готовая иерархия — то, из чего собирается снимок поиска.
    pub fn hierarchy(&self) -> Option<Arc<bevy_northstar::prelude::OrdinalGrid>> {
        match self {
            Self::Grid(GridMode::Hierarchy(grid)) => Some(grid.clone()),
            _ => None,
        }
    }

    /// Готовый меш — то же самое для полигонального бэкенда. `None` здесь уже
    /// не двусмысленно: «выключен» и «строится» различимы самим вариантом.
    pub fn mesh(&self) -> Option<Arc<PolymeshBuild>> {
        match self {
            Self::Mesh(MeshMode::Ready(mesh)) => Some(mesh.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Таблица истинности целиком — то, ради чего значение и заведено: раньше
    /// каждая её колонка собиралась в своём файле из своей комбинации
    /// ресурсов, и сверить их между собой было негде.
    #[test]
    fn the_truth_table_holds() {
        let flat = NavMode::Grid(GridMode::Flat);
        let wanted = NavMode::Grid(GridMode::HierarchyPending { wanted: true });
        let building = NavMode::Grid(GridMode::HierarchyPending { wanted: false });
        let pending = NavMode::Mesh(MeshMode::Pending);

        for (mode, continuous, is_building, northstar) in [
            (&flat, false, false, false),
            (&wanted, false, true, true),
            (&building, false, true, false),
            (&pending, true, true, false),
        ] {
            assert_eq!(mode.is_continuous(), continuous, "{mode:?}");
            assert_eq!(mode.is_building(), is_building, "{mode:?}");
            assert_eq!(mode.northstar_wanted(), northstar, "{mode:?}");
        }
    }

    /// Та самая клетка, ради которой значение и понадобилось: «выключен» и
    /// «ещё строится» больше не один и тот же `None`.
    #[test]
    fn an_unbuilt_mesh_is_not_the_same_as_no_mesh() {
        let off = NavMode::Grid(GridMode::Flat);
        let pending = NavMode::Mesh(MeshMode::Pending);

        assert!(off.mesh().is_none());
        assert!(pending.mesh().is_none());
        // …и при этом они различимы:
        assert!(!off.is_continuous());
        assert!(pending.is_continuous());
    }
}
