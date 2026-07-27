use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::tasks::Task;

use crate::navigation::{ArcNavmesh, PathfindingResult, astar_pathfinding};

#[derive(Debug, Clone, Eq, PartialEq, Default, Reflect)]
pub enum MovableState {
    #[default]
    Idle,
    /// Ждём ответа асинхронного поиска пути к тайлу.
    Pathfinding(IVec2),
    /// Идём по пути к тайлу.
    Moving(IVec2),
    /// Поиск пути не удался — поведение должно выбрать новую цель.
    PathfindingError(IVec2),
}

/// Тег для дешёвой выборки движущихся сущностей.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct MovableStateMovingTag;

/// Позиция сущности в симуляции — источник истины для движения. Двигается в
/// `FixedUpdate`; `Transform` — визуальное представление, интерполируемое
/// между фиксированными шагами.
#[derive(Component, Debug, Default, Deref, DerefMut, Reflect)]
#[reflect(Component)]
pub struct SimPosition(pub Vec2);

/// `SimPosition` на прошлом фиксированном шаге — второй конец интерполяции.
#[derive(Component, Debug, Default, Deref, DerefMut, Reflect)]
#[reflect(Component)]
pub struct PreviousSimPosition(pub Vec2);

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
#[require(SimPosition, PreviousSimPosition)]
pub struct Movable {
    pub speed: f32,
    pub path: VecDeque<IVec2>,
    pub state: MovableState,
}

/// Асинхронный поиск пути этой сущности. Один таск на сущность: новый запрос
/// вытесняет старый, а дроп `Task` его отменяет.
#[derive(Component, Debug)]
pub struct PathfindingTask(pub Task<PathfindingResult>);

#[derive(EntityEvent, Debug, Clone)]
pub struct MovableReachedDestinationEvent {
    pub entity: Entity,
    pub grid_tile: IVec2,
}

impl Movable {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            path: VecDeque::new(),
            state: MovableState::Idle,
        }
    }

    pub fn to_idle(&mut self, entity: Entity, commands: &mut Commands, destination_reached: bool) {
        if destination_reached
            && self.path.is_empty()
            && let MovableState::Moving(end_tile) | MovableState::Pathfinding(end_tile) = self.state
        {
            commands.trigger(MovableReachedDestinationEvent {
                entity,
                grid_tile: end_tile,
            });
        }

        self.stop_moving(entity, commands);
        self.state = MovableState::Idle;
    }

    pub fn to_moving(
        &mut self,
        end_tile: IVec2,
        path: VecDeque<IVec2>,
        entity: Entity,
        commands: &mut Commands,
    ) {
        self.state = MovableState::Moving(end_tile);
        self.path = path;
        commands.entity(entity).insert(MovableStateMovingTag);
    }

    /// Запустить асинхронный поиск пути; ответ снимает
    /// `listen_for_pathfinding_tasks`.
    pub fn to_pathfinding(
        &mut self,
        entity: Entity,
        start_tile: IVec2,
        end_tile: IVec2,
        arc_navmesh: &ArcNavmesh,
        commands: &mut Commands,
    ) {
        self.stop_moving(entity, commands);
        self.state = MovableState::Pathfinding(end_tile);

        let navmesh = arc_navmesh.0.clone();
        let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
            let navmesh = navmesh.read().unwrap();
            PathfindingResult {
                start_tile,
                end_tile,
                path: astar_pathfinding(&navmesh, start_tile, end_tile),
            }
        });

        // Вставка вытесняет прошлый таск этой сущности.
        commands.entity(entity).insert(PathfindingTask(task));
    }

    pub fn to_pathfinding_error(
        &mut self,
        entity: Entity,
        end_tile: IVec2,
        commands: &mut Commands,
    ) {
        self.stop_moving(entity, commands);
        self.state = MovableState::PathfindingError(end_tile);
    }

    fn stop_moving(&mut self, entity: Entity, commands: &mut Commands) {
        self.path = [].into();
        commands.entity(entity).remove::<MovableStateMovingTag>();
    }
}
