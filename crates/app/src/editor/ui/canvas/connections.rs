//! The routes of all connections, kept up to date across frames.
//!
//! [`RouteCache`] lives in the app (it is derived data, never saved) rather than in
//! egui memory, so it is plain Rust that unit tests can drive without a UI.

use eframe::egui;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use super::CONNECTION_MIN_HANDLE;
use crate::editor::ui::routing::{self, Route};
use crate::editor::{NodeWidget, SocketRef};

/// Time per frame for recomputing connection routes (see [`RouteCache::update`]);
/// the rest wait for the next frames.
pub(super) const ROUTING_BUDGET: Duration = Duration::from_millis(4);

/// One connection's route, with what later connections need to route around it.
pub(super) struct RoutedConnection {
    /// The input socket (the connection's key in `SignalNodesApp::connections`).
    pub(super) to: SocketRef,
    /// The output socket feeding `to`.
    pub(super) from: SocketRef,
    /// Hash of everything the route was computed from (see [`RouteCache::update`]).
    key: u64,
    /// A stand-in (the plain curve) for a new connection that wasn't routed yet
    /// because the frame's budget ran out; replaced as soon as possible.
    provisional: bool,
    /// How to draw the connection, in canvas coordinates.
    pub(super) route: Route,
    /// The route as a polyline, a wire for the connections routed after it.
    polyline: Vec<egui::Pos2>,
    /// Hash of `polyline`: later connections' keys depend on this, not on `key`, so
    /// recomputing a route that comes out the same doesn't reroute them.
    shape: u64,
    /// Bounds of `polyline`.
    bounds: egui::Rect,
}

impl RoutedConnection {
    fn new(to: SocketRef, from: SocketRef, key: u64, route: Route, provisional: bool) -> Self {
        let polyline = route.polyline();
        let mut hasher = DefaultHasher::new();
        hash_points(&mut hasher, &polyline);
        Self {
            to,
            from,
            key,
            provisional,
            route,
            shape: hasher.finish(),
            bounds: egui::Rect::from_points(&polyline),
            polyline,
        }
    }
}

/// What one [`RouteCache::update`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RouteUpdate {
    /// Routes computed afresh.
    pub(super) recomputed: usize,
    /// Outdated or new routes left for a later update because the budget ran out.
    pub(super) deferred: usize,
}

/// Routes of all connections, in a fixed order (by source, then target), which is
/// also the paint order.
///
/// Connections are routed one after another, each against the routes before it and
/// the stubs of all the others (see the `routing` module docs), so different
/// sources' connections avoid crossing where they can.
#[derive(Default)]
pub(in crate::editor) struct RouteCache {
    /// Routes as of the last [`update`](Self::update), in routing order.
    routes: Vec<RoutedConnection>,
}

impl RouteCache {
    /// The routes as of the last [`update`](Self::update), in paint order.
    pub(super) fn routes(&self) -> &[RoutedConnection] {
        &self.routes
    }

    /// Brings the routes up to date with `nodes` and `connections`
    /// (`SignalNodesApp::connections`: input → output).
    ///
    /// Routing runs a path search per blocked connection, so routes are kept per
    /// connection. A route's key hashes its ends and the nodes, stubs and earlier
    /// routes' shapes (polylines) within its [`routing::influence`] area; it is
    /// recomputed only when that key changes. Dragging a node reroutes only the
    /// connections near it, and those after them only if a nearby route actually
    /// changed shape, not every connection on the canvas.
    ///
    /// Recomputing stops once it has taken `budget` (after at least one route): the
    /// remaining outdated connections keep their old route and new ones get the
    /// plain curve, both counted as [`deferred`](RouteUpdate::deferred), for the next
    /// update to finish. During a long drag the last connections in the order may
    /// therefore catch up only once the drag stops.
    pub(super) fn update(
        &mut self,
        nodes: &[NodeWidget],
        connections: &HashMap<SocketRef, SocketRef>,
        budget: Duration,
    ) -> RouteUpdate {
        let socket_pos = |socket: SocketRef| {
            nodes
                .iter()
                .find(|node| node.id == socket.node_id)
                .map(|node| node.socket_pos(socket.side, socket.index))
        };
        // Sorted by ID, so raising a node (which only reorders `nodes`) changes no key.
        let mut node_rects: Vec<_> = nodes.iter().map(|node| (node.id, node.rect())).collect();
        node_rects.sort_by_key(|(id, _)| *id);

        let mut ends: Vec<_> = connections
            .iter()
            .filter_map(|(to, from)| Some((*to, *from, socket_pos(*from)?, socket_pos(*to)?)))
            .collect();
        ends.sort_by_key(|(to, from, ..)| (from.node_id, from.index, to.node_id, to.index));
        let stubs: Vec<_> = ends
            .iter()
            .flat_map(|(_, from, start, end)| {
                routing::stubs(*start, *end).map(|stub| (*from, stub))
            })
            .collect();

        // Routes of connections that are gone are dropped with `previous`.
        let mut previous: HashMap<SocketRef, RoutedConnection> = std::mem::take(&mut self.routes)
            .into_iter()
            .map(|routed| (routed.to, routed))
            .collect();
        let mut routed: Vec<RoutedConnection> = Vec::with_capacity(ends.len());
        let mut update = RouteUpdate::default();
        let started = Instant::now();
        for &(to, from, start, end) in &ends {
            let area = routing::influence(start, end, CONNECTION_MIN_HANDLE);
            let obstacles: Vec<_> = node_rects
                .iter()
                .map(|(_, rect)| *rect)
                .filter(|rect| rect.intersects(area))
                .collect();
            let mut hasher = DefaultHasher::new();
            hash_points(&mut hasher, &[start, end]);
            for rect in &obstacles {
                hash_points(&mut hasher, &[rect.min, rect.max]);
            }
            // Wires from the same output are left out: fanning out may share a stretch.
            let mut wires: Vec<&[egui::Pos2]> = Vec::new();
            for (_, stub) in stubs.iter().filter(|(source, stub)| {
                *source != from && egui::Rect::from_two_pos(stub[0], stub[1]).intersects(area)
            }) {
                hash_points(&mut hasher, stub);
                wires.push(stub);
            }
            for earlier in routed
                .iter()
                .filter(|earlier| earlier.from != from && earlier.bounds.intersects(area))
            {
                earlier.shape.hash(&mut hasher);
                wires.push(&earlier.polyline);
            }
            let key = hasher.finish();

            let cached = previous.remove(&to).filter(|cached| cached.from == from);
            let connection = match cached {
                Some(cached) if cached.key == key && !cached.provisional => cached,
                cached if update.recomputed > 0 && started.elapsed() >= budget => {
                    update.deferred += 1;
                    cached.unwrap_or_else(|| {
                        let curve = routing::bezier(start, end, CONNECTION_MIN_HANDLE);
                        RoutedConnection::new(to, from, key, Route::Curve(curve), true)
                    })
                }
                _ => {
                    update.recomputed += 1;
                    let route =
                        routing::route(start, end, &obstacles, &wires, CONNECTION_MIN_HANDLE);
                    RoutedConnection::new(to, from, key, route, false)
                }
            };
            routed.push(connection);
        }
        self.routes = routed;
        update
    }
}

/// Hashes the bits of `points`, so equal layouts give equal keys.
fn hash_points(hasher: &mut DefaultHasher, points: &[egui::Pos2]) {
    for point in points {
        [point.x, point.y].map(f32::to_bits).hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{NodeConfig, NodeKind, SocketKind};

    fn node(id: usize, kind: NodeKind, x: f32, y: f32) -> NodeWidget {
        NodeWidget::new(id, NodeConfig::for_kind(kind), egui::pos2(x, y))
    }

    fn socket(node_id: usize, side: SocketKind, index: usize) -> SocketRef {
        SocketRef {
            node_id,
            side,
            index,
        }
    }

    /// Source 0 feeding sink 2 around gain 1 in the way, and source 3 feeding the gain.
    fn layout() -> (Vec<NodeWidget>, HashMap<SocketRef, SocketRef>) {
        let nodes = vec![
            node(0, NodeKind::Source, 0.0, 0.0),
            node(1, NodeKind::Gain, 250.0, 0.0),
            node(2, NodeKind::Sink, 500.0, 0.0),
            node(3, NodeKind::Source, 0.0, 150.0),
        ];
        let connections = HashMap::from([
            (
                socket(2, SocketKind::Input, 0),
                socket(0, SocketKind::Output, 0),
            ),
            (
                socket(1, SocketKind::Input, 0),
                socket(3, SocketKind::Output, 0),
            ),
        ]);
        (nodes, connections)
    }

    fn routes(cache: &RouteCache) -> Vec<(SocketRef, Route)> {
        cache
            .routes()
            .iter()
            .map(|routed| (routed.to, routed.route.clone()))
            .collect()
    }

    #[test]
    fn routes_are_reused_until_something_nearby_moves() {
        let (mut nodes, connections) = layout();
        let mut cache = RouteCache::default();

        let first = cache.update(&nodes, &connections, Duration::MAX);
        assert_eq!(first.recomputed, 2);
        assert!(matches!(cache.routes()[0].route, Route::Path(_)));
        let routed = routes(&cache);

        // Nothing changed: nothing is recomputed.
        let again = cache.update(&nodes, &connections, Duration::MAX);
        assert_eq!(again, RouteUpdate::default());
        assert_eq!(routes(&cache), routed);

        // A node far outside every influence area doesn't matter either.
        nodes.push(node(4, NodeKind::Gain, 5_000.0, 5_000.0));
        assert_eq!(
            cache.update(&nodes, &connections, Duration::MAX).recomputed,
            0
        );

        // Moving the gain in the way reroutes.
        nodes[1].pos.y += 40.0;
        assert!(cache.update(&nodes, &connections, Duration::MAX).recomputed > 0);
    }

    #[test]
    fn an_exhausted_budget_defers_routes_to_later_updates() {
        let (nodes, connections) = layout();
        let mut full = RouteCache::default();
        full.update(&nodes, &connections, Duration::MAX);

        let mut cache = RouteCache::default();
        let first = cache.update(&nodes, &connections, Duration::ZERO);
        // At least one route per update, the other one stands in as the curve.
        assert_eq!(
            first,
            RouteUpdate {
                recomputed: 1,
                deferred: 1
            }
        );
        assert_eq!(cache.routes().len(), 2);
        assert!(cache.routes()[1].provisional);

        let second = cache.update(&nodes, &connections, Duration::ZERO);
        assert_eq!(second.deferred, 0);
        assert_eq!(routes(&cache), routes(&full));
    }

    #[test]
    fn removed_connections_are_dropped() {
        let (nodes, mut connections) = layout();
        let mut cache = RouteCache::default();
        cache.update(&nodes, &connections, Duration::MAX);

        connections.remove(&socket(2, SocketKind::Input, 0));
        cache.update(&nodes, &connections, Duration::MAX);

        assert_eq!(cache.routes().len(), 1);
        assert_eq!(cache.routes()[0].to, socket(1, SocketKind::Input, 0));
    }
}
