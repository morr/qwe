#[cfg(feature = "stats")]
use std::time::Instant;
use std::{collections::HashSet, fmt, future::Future, task::Poll};

use crate::{
    instance::{InstanceStep, SearchInstance},
    Coords, Mesh, Path,
};

/// A future that will resolve to a [`Option<Path>`].
///
/// This will be a [`Path`] if a path is found, or `None` if not. Returned by [`Mesh::get_path`]
/// and [`Mesh::get_path_on_layers`].
pub struct FuturePath<'m> {
    pub(crate) from: Coords,
    pub(crate) to: Coords,
    pub(crate) mesh: &'m Mesh,
    pub(crate) instance: Option<SearchInstance<'m>>,
    pub(crate) ending_polygon: u32,
    /// Layers the search is not allowed to enter; the counterpart of
    /// [`Mesh::path_on_layers`] for the polled search.
    pub(crate) blocked_layers: HashSet<u8>,
}

impl fmt::Debug for FuturePath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FuturePath")
            .field("from", &self.from.position())
            .field("to", &self.to.position())
            .finish()
    }
}

impl FuturePath<'_> {
    /// Polygon for one end of the search: the one already found by a previous
    /// mesh search if the [`Coords`] carries it, otherwise located the same way
    /// [`Mesh::path_on_layers`] does — honoring the blocked layers, which plain
    /// [`Mesh::get_point_location`] knows nothing about.
    fn end_polygon(&self, point: &Coords) -> u32 {
        if point.polygon_index != u32::MAX {
            return point.polygon_index;
        }
        if self.blocked_layers.is_empty() {
            return self.mesh.get_point_location(point.pos);
        }
        self.mesh
            .get_closest_point_on_layers(*point, self.blocked_layers.clone())
            .map_or(u32::MAX, |located| located.polygon_index)
    }
}

impl Future for FuturePath<'_> {
    type Output = Option<Path>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if let Some(search_instance) = self.instance.as_mut() {
            for _i in 0..3 {
                match search_instance.next() {
                    InstanceStep::Found(path) => return Poll::Ready(Some(path)),
                    InstanceStep::NotFound => return Poll::Ready(None),
                    InstanceStep::Continue => {}
                }
            }
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            #[cfg(feature = "stats")]
            let start = Instant::now();

            let starting_polygon_index = self.end_polygon(&self.from);
            if starting_polygon_index == u32::MAX {
                return Poll::Ready(None);
            }
            let ending_polygon = self.end_polygon(&self.to);
            if ending_polygon == u32::MAX {
                return Poll::Ready(None);
            }
            // Islands are per layer; on a multi-layer mesh a same-layer pair may
            // still be connected through another layer — same gate as
            // `Mesh::path_on_layers`.
            if self.mesh.layers.len() == 1 {
                if let Some(islands) = self.mesh.layers[0].islands.as_ref() {
                    let start_island = islands.get(starting_polygon_index as usize);
                    let end_island = islands.get(ending_polygon as usize);
                    if start_island.is_some() && end_island.is_some() && start_island != end_island
                    {
                        return Poll::Ready(None);
                    }
                }
            }

            if starting_polygon_index == ending_polygon {
                #[cfg(feature = "stats")]
                {
                    if self.mesh.scenarios.get() == 0 {
                        eprintln!(
                        "index;micros;successor_calls;generated;pushed;popped;pruned_post_pop;length",
                    );
                    }
                    eprintln!(
                        "{};{};0;0;0;0;0;{}",
                        self.mesh.scenarios.get(),
                        start.elapsed().as_secs_f32() * 1_000_000.0,
                        self.from.pos.distance(self.to.pos),
                    );
                    self.mesh.scenarios.set(self.mesh.scenarios.get() + 1);
                }
                return Poll::Ready(Some(Path {
                    length: self.from.pos.distance(self.to.pos),
                    path: vec![self.to.pos],
                    #[cfg(feature = "detailed-layers")]
                    #[cfg_attr(docsrs, doc(cfg(feature = "detailed-layers")))]
                    path_with_layers: vec![(self.to.pos, crate::instance::U32Layer::layer(&ending_polygon))],
                    path_through_polygons: vec![ending_polygon],
                }));
            }

            let blocked_layers = self.blocked_layers.clone();
            self.instance = Some(SearchInstance::setup(
                self.mesh,
                (self.from.pos, starting_polygon_index),
                (self.to.pos, ending_polygon),
                blocked_layers,
                #[cfg(feature = "stats")]
                start,
            ));
            self.ending_polygon = ending_polygon;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
