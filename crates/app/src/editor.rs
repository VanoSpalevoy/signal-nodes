//! Editor state and the `eframe` application entry.
//!
//! [`SignalNodesApp`] owns the whole graph being edited (nodes and connections) and
//! the transient UI state (socket selection, language, zoom). Rendering and input
//! handling live in the [`ui`] module, which mutates this state directly. Graph
//! edits that have rules (adding, removing, connecting) are methods here, so they
//! can be unit-tested without egui.

use eframe::egui;
use std::collections::{HashMap, HashSet};

mod localization;
mod ui;

use localization::{Language, Text};
use ui::{NodeConfig, NodeKind, NodeWidget, RouteCache, SocketKind, SocketRef};

/// Maximum number of nodes on the canvas; also the size of the node ID pool.
const MAX_NODE_COUNT: usize = 256;

/// The node editor.
///
/// Invariants that every add/remove path must keep:
/// - each ID in `0..MAX_NODE_COUNT` is either used by exactly one node in `nodes`
///   or sits in `free_node_ids`, never both;
/// - `connections` and `pending_source` only refer to nodes that exist;
/// - `node_limit_reached` is set only by a failed [`add_node_at`](Self::add_node_at)
///   and cleared by the next successful add or any deletion;
/// - `connections` never contains a cycle (enforced by [`connect`](Self::connect));
/// - nodes never overlap: adds go through [`add_node_at`](Self::add_node_at), drags
///   through [`drag_to`](Self::drag_to) and snapping through
///   [`snap_to_grid`](Self::snap_to_grid), all using `ui::placement`;
/// - node positions change only through those methods (and
///   [`cancel_drag`](Self::cancel_drag)), never by the UI writing `pos` itself;
/// - `drag` only refers to a node that exists.
pub struct SignalNodesApp {
    /// Nodes in paint order: later nodes are drawn on top and win overlapping clicks.
    nodes: Vec<NodeWidget>,
    /// Graph edges, keyed by the **input** socket with the output socket as value,
    /// so each input has at most one source while an output can feed many inputs.
    connections: HashMap<SocketRef, SocketRef>,
    /// Unused node IDs, used as a stack: deleted IDs are pushed back and reused first.
    free_node_ids: Vec<usize>,
    /// The last attempt to add a node failed because the ID pool is empty.
    /// Shown as a warning in the debug panel.
    node_limit_reached: bool,
    /// Output socket picked by a primary click, waiting for an input to connect to.
    pending_source: Option<SocketRef>,
    /// The last connection attempt was refused because it would create a cycle.
    /// Shown as a warning in the debug panel; cleared by the next successful
    /// connection, disconnection or source selection.
    connection_refused: bool,
    /// Node kind clicked in the palette this frame; the canvas adds it in the
    /// middle of the visible area and clears it. (Palette drags don't use this:
    /// they travel as an egui drag-and-drop payload.)
    requested_node: Option<NodeKind>,
    /// UI language; every visible string is resolved through it on each frame.
    language: Language,
    /// Canvas zoom chosen in the toolbar: 1.0 = one canvas unit per screen point.
    scale: f32,
    /// The node being dragged on the canvas, if any.
    drag: Option<NodeDrag>,
    /// Connection routes, derived from `nodes` and `connections` and brought up to
    /// date by the canvas every frame. Never saved.
    routes: RouteCache,
}

/// A node drag in progress, from [`begin_drag`](SignalNodesApp::begin_drag) to
/// [`end_drag`](SignalNodesApp::end_drag): the whole gesture is one move from
/// `from` to wherever the node ends up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeDrag {
    node_id: usize,
    /// Where the node was when the drag began, canvas coordinates.
    from: egui::Pos2,
}

impl Default for SignalNodesApp {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            connections: HashMap::new(),
            free_node_ids: (0..MAX_NODE_COUNT).rev().collect(),
            node_limit_reached: false,
            pending_source: None,
            connection_refused: false,
            requested_node: None,
            language: Language::English,
            scale: 1.0,
            drag: None,
            routes: RouteCache::default(),
        }
    }
}

impl SignalNodesApp {
    /// Creates the editor and installs its theme once for the app's lifetime.
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        ui::install_theme(&creation_context.egui_ctx);
        Self::default()
    }

    /// Adds a node of `kind` with its top-left corner as close to `pos` (canvas
    /// coordinates) as possible without overlapping other nodes.
    ///
    /// When all [`MAX_NODE_COUNT`] IDs are in use, nothing is added and
    /// `node_limit_reached` is set instead.
    fn add_node_at(&mut self, kind: NodeKind, pos: egui::Pos2) {
        let Some(id) = self.free_node_ids.pop() else {
            self.node_limit_reached = true;
            return;
        };
        self.node_limit_reached = false;
        let config = NodeConfig::for_kind(kind);
        let pos = ui::place_new_node(&self.nodes, &config, pos);
        self.nodes.push(NodeWidget::new(id, config, pos));
    }

    /// Removes node `node_id` together with every connection to or from it,
    /// returns its ID to the pool and drops a pending source on it.
    /// Does nothing if there is no such node.
    fn remove_node(&mut self, node_id: usize) {
        let Some(index) = self.nodes.iter().position(|node| node.id == node_id) else {
            return;
        };
        self.nodes.remove(index);
        self.connections
            .retain(|input, output| input.node_id != node_id && output.node_id != node_id);
        self.free_node_ids.push(node_id);
        self.node_limit_reached = false;
        if self.drag.is_some_and(|drag| drag.node_id == node_id) {
            self.drag = None;
        }
        if self
            .pending_source
            .is_some_and(|source| source.node_id == node_id)
        {
            self.pending_source = None;
        }
    }

    /// Moves node `node_id` to the end of `nodes`, so it is drawn above the others
    /// and wins overlapping clicks.
    fn raise_node(&mut self, node_id: usize) {
        if let Some(index) = self.nodes.iter().position(|node| node.id == node_id) {
            let node = self.nodes.remove(index);
            self.nodes.push(node);
        }
    }

    /// Starts dragging node `node_id` (ending any other drag first). Does nothing
    /// if there is no such node.
    fn begin_drag(&mut self, node_id: usize) {
        self.end_drag();
        if let Some(node) = self.nodes.iter().find(|node| node.id == node_id) {
            self.drag = Some(NodeDrag {
                node_id,
                from: node.pos,
            });
        }
    }

    /// The node being dragged, if any.
    fn dragged_node(&self) -> Option<usize> {
        self.drag.map(|drag| drag.node_id)
    }

    /// Moves the dragged node `node_id` towards `target` (its new top-left corner,
    /// canvas coordinates). It slides and stops at the first node in the way
    /// instead of overlapping it (`placement::slide`), and never goes negative.
    ///
    /// Does nothing unless `node_id` is the node being dragged, so the rest of a
    /// cancelled drag gesture is ignored.
    fn drag_to(&mut self, node_id: usize, target: egui::Pos2) {
        if self.dragged_node() != Some(node_id) {
            return;
        }
        let obstacles: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.id != node_id)
            .map(NodeWidget::rect)
            .collect();
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            node.pos = ui::slide(node.rect(), target, &obstacles);
        }
    }

    /// Finishes the drag in progress: the node stays where it is. This is where
    /// the whole drag is known as one move (`from` → current position), e.g. for
    /// an undo step.
    fn end_drag(&mut self) {
        self.drag = None;
    }

    /// Cancels the drag in progress and puts the node back where the drag began.
    /// (That spot is still free: only the dragged node moved since.)
    fn cancel_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == drag.node_id) {
            node.pos = drag.from;
        }
    }

    /// Snaps every node to the nearest point of a `step` grid that keeps clear of
    /// the others. The grid starts at the canvas origin. Nodes are placed one by
    /// one, each against the current positions of all the others.
    fn snap_to_grid(&mut self, step: f32) {
        for index in 0..self.nodes.len() {
            let obstacles: Vec<_> = self
                .nodes
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, node)| node.rect())
                .collect();
            let node = &mut self.nodes[index];
            let snapped = (node.pos / step).round() * step;
            node.pos = ui::free_pos(snapped, node.rect().size(), &obstacles, step);
        }
    }

    /// Connects output socket `from` to input socket `to`, replacing whatever fed
    /// `to` before. Returns whether the connection was made.
    ///
    /// Refused (and `connection_refused` set) when the new edge would close a cycle,
    /// including connecting a node to itself: the graph must stay acyclic to be
    /// processed in order.
    fn connect(&mut self, from: SocketRef, to: SocketRef) -> bool {
        debug_assert!(from.side == SocketKind::Output && to.side == SocketKind::Input);
        if self.reaches(to.node_id, from.node_id) {
            self.connection_refused = true;
            return false;
        }
        self.connection_refused = false;
        self.connections.insert(to, from);
        true
    }

    /// Removes the connection into input socket `to`, if any.
    fn disconnect_input(&mut self, to: SocketRef) {
        debug_assert!(to.side == SocketKind::Input);
        self.connections.remove(&to);
        self.connection_refused = false;
    }

    /// Removes every connection from output socket `from`.
    fn disconnect_output(&mut self, from: SocketRef) {
        debug_assert!(from.side == SocketKind::Output);
        self.connections.retain(|_, source| *source != from);
        self.connection_refused = false;
    }

    /// Whether `target` can be reached from `start` by following connections
    /// downstream (output → input). A node reaches itself.
    fn reaches(&self, start: usize, target: usize) -> bool {
        let mut stack = vec![start];
        let mut visited = HashSet::new();
        while let Some(node_id) = stack.pop() {
            if node_id == target {
                return true;
            }
            if visited.insert(node_id) {
                stack.extend(
                    self.connections
                        .iter()
                        .filter(|(_, from)| from.node_id == node_id)
                        .map(|(to, _)| to.node_id),
                );
            }
        }
        false
    }
}

impl eframe::App for SignalNodesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::render(self, ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(node_id: usize) -> SocketRef {
        SocketRef {
            node_id,
            side: SocketKind::Output,
            index: 0,
        }
    }

    fn input(node_id: usize, index: usize) -> SocketRef {
        SocketRef {
            node_id,
            side: SocketKind::Input,
            index,
        }
    }

    #[test]
    fn connect_replaces_existing_source() {
        let mut app = SignalNodesApp::default();

        assert!(app.connect(output(0), input(2, 0)));
        assert!(app.connect(output(1), input(2, 0)));

        assert_eq!(app.connections.len(), 1);
        assert_eq!(app.connections[&input(2, 0)], output(1));
    }

    #[test]
    fn connect_refuses_cycles() {
        let mut app = SignalNodesApp::default();

        assert!(!app.connect(output(0), input(0, 0)));
        assert!(app.connection_refused);

        assert!(app.connect(output(0), input(1, 0)));
        assert!(app.connect(output(1), input(2, 0)));
        assert!(!app.connect(output(2), input(0, 0)));
        assert!(app.connection_refused);
        assert_eq!(app.connections.len(), 2);

        // A diamond is not a cycle.
        assert!(app.connect(output(0), input(2, 1)));
        assert!(!app.connection_refused);
    }

    #[test]
    fn disconnect_input_removes_only_that_input() {
        let mut app = SignalNodesApp::default();
        app.connect(output(0), input(2, 0));
        app.connect(output(1), input(2, 1));
        app.connection_refused = true;

        app.disconnect_input(input(2, 0));

        assert_eq!(app.connections.len(), 1);
        assert_eq!(app.connections[&input(2, 1)], output(1));
        assert!(!app.connection_refused);
    }

    #[test]
    fn disconnect_output_removes_every_connection_from_it() {
        let mut app = SignalNodesApp::default();
        app.connect(output(0), input(1, 0));
        app.connect(output(0), input(2, 0));
        app.connect(output(1), input(2, 1));
        app.connection_refused = true;

        app.disconnect_output(output(0));

        assert_eq!(app.connections.len(), 1);
        assert_eq!(app.connections[&input(2, 1)], output(1));
        assert!(!app.connection_refused);
    }

    #[test]
    fn remove_node_cleans_up_and_frees_id() {
        let mut app = SignalNodesApp::default();
        for _ in 0..3 {
            app.add_node_at(NodeKind::Gain, egui::Pos2::ZERO);
        }
        let ids: Vec<_> = app.nodes.iter().map(|node| node.id).collect();
        app.connect(output(ids[0]), input(ids[1], 0));
        app.connect(output(ids[1]), input(ids[2], 0));
        app.pending_source = Some(output(ids[1]));

        app.remove_node(ids[1]);

        assert_eq!(app.nodes.len(), 2);
        assert!(app.connections.is_empty());
        assert_eq!(app.pending_source, None);
        // The freed ID is reused first.
        app.add_node_at(NodeKind::Sink, egui::Pos2::ZERO);
        assert_eq!(app.nodes.last().unwrap().id, ids[1]);
    }

    #[test]
    fn nodes_added_at_the_same_spot_do_not_overlap() {
        let mut app = SignalNodesApp::default();
        for _ in 0..5 {
            app.add_node_at(NodeKind::Mixer, egui::pos2(100.0, 100.0));
        }

        for (i, a) in app.nodes.iter().enumerate() {
            for b in &app.nodes[i + 1..] {
                assert!(
                    !a.rect().intersects(b.rect()),
                    "nodes {} and {} overlap",
                    a.id,
                    b.id
                );
            }
        }
    }

    /// Two gain nodes side by side, `NODE_GAP`-ish apart: ids 0 (left) and 1.
    fn two_nodes() -> SignalNodesApp {
        let mut app = SignalNodesApp::default();
        app.add_node_at(NodeKind::Gain, egui::pos2(0.0, 0.0));
        app.add_node_at(NodeKind::Gain, egui::pos2(300.0, 0.0));
        app
    }

    fn pos(app: &SignalNodesApp, node_id: usize) -> egui::Pos2 {
        app.nodes
            .iter()
            .find(|node| node.id == node_id)
            .unwrap()
            .pos
    }

    #[test]
    fn dragging_moves_the_node_but_not_into_another() {
        let mut app = two_nodes();
        app.begin_drag(0);

        app.drag_to(0, egui::pos2(50.0, 20.0));
        assert_eq!(pos(&app, 0), egui::pos2(50.0, 20.0));

        // Pushed onto node 1: stops in front of it.
        app.drag_to(0, egui::pos2(300.0, 20.0));
        assert!(!app.nodes[0].rect().intersects(app.nodes[1].rect()));
        assert!(pos(&app, 0).x < 300.0);

        app.end_drag();
        assert_eq!(app.dragged_node(), None);
        // Outside a drag, moves are ignored.
        app.drag_to(0, egui::pos2(0.0, 0.0));
        assert!(pos(&app, 0).x > 50.0);
    }

    #[test]
    fn cancelling_a_drag_puts_the_node_back_and_ignores_the_rest() {
        let mut app = two_nodes();
        app.begin_drag(1);
        app.drag_to(1, egui::pos2(400.0, 300.0));

        app.cancel_drag();
        assert_eq!(pos(&app, 1), egui::pos2(300.0, 0.0));
        assert_eq!(app.dragged_node(), None);

        app.drag_to(1, egui::pos2(500.0, 500.0));
        assert_eq!(pos(&app, 1), egui::pos2(300.0, 0.0));
    }

    #[test]
    fn removing_the_dragged_node_ends_the_drag() {
        let mut app = two_nodes();
        app.begin_drag(0);

        app.remove_node(0);

        assert_eq!(app.drag, None);
        app.cancel_drag();
        assert_eq!(app.nodes.len(), 1);
    }

    #[test]
    fn snap_to_grid_aligns_nodes_without_overlap() {
        let mut app = two_nodes();
        app.nodes[0].pos = egui::pos2(13.0, 38.0);
        app.nodes[1].pos = egui::pos2(187.0, 61.0);

        app.snap_to_grid(50.0);

        for node in &app.nodes {
            assert_eq!(node.pos.x % 50.0, 0.0);
            assert_eq!(node.pos.y % 50.0, 0.0);
        }
        assert!(!app.nodes[0].rect().intersects(app.nodes[1].rect()));
    }

    #[test]
    fn raise_node_moves_it_to_the_top() {
        let mut app = SignalNodesApp::default();
        for _ in 0..3 {
            app.add_node_at(NodeKind::Gain, egui::Pos2::ZERO);
        }
        let bottom = app.nodes[0].id;

        app.raise_node(bottom);

        assert_eq!(app.nodes.last().unwrap().id, bottom);
        assert_eq!(app.nodes.len(), 3);
    }
}
