//! Досыпка крон-сущностей: [`stream_tree_crowns`] спавнит их пачками
//! спрятанными, показывает ступень последней пачкой и убирает кроны с дальних
//! ступеней из мира, а не держит спрятанными.

use super::super::mesh::CROWN_SPAWN_BATCH;
use super::super::*;
use crate::map::z::Z_TREE;

/// Кроны-сущности ближней ступени встают пачками, спрятанными, и показанная
/// ступень ждёт последней пачки; на дальней ступени они прячутся сразу и
/// уходят из мира, а не стоят спрятанными — у тех своя цена каждого кадра.
#[test]
fn crown_entities_stream_in_by_batches_and_leave_on_the_far_steps() {
    use bevy::ecs::system::RunSystemOnce;

    let crowns = |world: &mut World| {
        world
            .query_filtered::<&Visibility, With<TreeTag>>()
            .iter(world)
            .copied()
            .collect::<Vec<_>>()
    };
    let shown = |world: &World| world.resource::<TreeLodShown>().0;
    let near = TreeLodMask::of([0]);
    let total = CROWN_SPAWN_BATCH + 10;
    let mut world = World::new();
    world.insert_resource(CrownStream {
        placements: (0..total)
            .map(|index| CrownPlacement {
                at: Vec2::new(index as f32, 0.0),
                radius: 1.0,
                z: Z_TREE,
                shows: near,
                pool: 0,
                variant: 0,
                tint: 0,
            })
            .collect(),
        pools: vec![vec![Handle::default()]],
        tints: vec![Handle::default()],
        entities: Vec::new(),
    });
    world.insert_resource(TreeZoomBucket::at(1));
    world.insert_resource(TreeLodShown(1));
    let step = |world: &mut World| world.run_system_once(stream_tree_crowns).unwrap();

    step(&mut world);
    assert!(
        crowns(&mut world).is_empty(),
        "дальняя ступень кроны не ставит"
    );

    world.insert_resource(TreeZoomBucket::at(0));
    step(&mut world);
    let first = crowns(&mut world);
    assert_eq!(first.len(), CROWN_SPAWN_BATCH);
    assert!(
        first
            .iter()
            .all(|visibility| *visibility == Visibility::Hidden)
    );
    assert_eq!(shown(&world), 1, "ступень показана до последней пачки");

    step(&mut world);
    assert_eq!(crowns(&mut world).len(), total);
    assert_eq!(shown(&world), 0);

    world.insert_resource(TreeZoomBucket::at(1));
    step(&mut world);
    assert_eq!(shown(&world), 1, "дальняя ступень показывается сразу");
    assert!(
        crowns(&mut world).is_empty(),
        "спрятанные кроны уходят из мира"
    );
}
