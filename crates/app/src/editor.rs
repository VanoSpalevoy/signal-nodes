//! Editor state and the `eframe` application entry.
//!
//! [`VSignalNodesApp`] owns the whole graph being edited (nodes and connections) and
//! the transient UI state (socket selection, language, zoom). Rendering and input
//! handling live in the [`ui`] module, which mutates this state directly. Graph
//! edits that have rules (adding, removing, connecting) are methods here, so they
//! can be unit-tested without egui.

use eframe::egui;
use std::collections::{HashMap, HashSet};
use v_signal_nodes_plugin_host::PluginHost;

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
/// - nodes never overlap, except the dragged node while the drag lasts: adds go
///   through [`add_node_at`](Self::add_node_at), drops through
///   [`end_drag`](Self::end_drag) and snapping through
///   [`snap_to_grid`](Self::snap_to_grid), all using `ui::placement`;
/// - node positions change only through those methods (and
///   [`drag_to`](Self::drag_to), [`cancel_drag`](Self::cancel_drag)), never by the
///   UI writing `pos` itself;
/// - `drag` only refers to a node that exists.
pub struct VSignalNodesApp {
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
    /// Loaded plugins, the source of every node type; nodes are built here (as
    /// `core::Node`s) before they become widgets.
    plugins: PluginHost,
    /// Every node type of `plugins`, in palette order (see [`palette`]).
    palette: Vec<NodeKind>,
}

/// A node drag in progress, from [`begin_drag`](VSignalNodesApp::begin_drag) to
/// [`end_drag`](VSignalNodesApp::end_drag): the whole gesture is one move from
/// `from` to wherever the node ends up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeDrag {
    node_id: usize,
    /// Where the node was when the drag began, canvas coordinates.
    from: egui::Pos2,
}

impl Default for VSignalNodesApp {
    fn default() -> Self {
        let plugins = builtin_plugins();
        let palette = palette(&plugins);
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
            plugins,
            palette,
        }
    }
}

/// A plugin host with the plugins linked into the editor: `std` and the `stub`
/// placeholder. They are part of the build, so failing to load one is a bug.
fn builtin_plugins() -> PluginHost {
    let mut host = PluginHost::new();
    host.load(&v_signal_nodes_plugin_std::StdPlugin)
        .expect("the std plugin loads");
    host.load(&v_signal_nodes_plugin_stub::StubPlugin)
        .expect("the stub plugin loads");
    host
}

/// Every node type of `plugins`, in palette order: generators, then processors,
/// then outputs (the order a signal flows), by type id within each group.
fn palette(plugins: &PluginHost) -> Vec<NodeKind> {
    use v_signal_nodes_core::NodeKind as Role;
    let mut kinds: Vec<_> = plugins.specs().cloned().map(NodeKind::new).collect();
    // `specs` is already sorted by type id and the sort is stable.
    kinds.sort_by_key(|kind| match kind.spec().kind {
        Role::Generator => 0,
        Role::Processor => 1,
        Role::Output => 2,
    });
    kinds
}

impl VSignalNodesApp {
    /// Creates the editor and installs its theme once for the app's lifetime.
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        ui::install_theme(&creation_context.egui_ctx);
        Self::default()
    }

    /// Adds a node of `kind` with its top-left corner as close to `pos` (canvas
    /// coordinates) as possible without overlapping other nodes.
    ///
    /// The node is built by `plugins` first (a `core::Node` with its processor)
    /// and its widget made from that node. The processor is dropped: there is no
    /// runtime to run it yet. If the host can't build the node (an unknown type
    /// id), nothing is added.
    ///
    /// When all [`MAX_NODE_COUNT`] IDs are in use, nothing is added and
    /// `node_limit_reached` is set instead.
    fn add_node_at(&mut self, kind: NodeKind, pos: egui::Pos2) {
        let Some(id) = self.free_node_ids.pop() else {
            self.node_limit_reached = true;
            return;
        };
        let config = match self.plugins.build_node(&kind.spec().type_id, id) {
            Ok(built) => NodeConfig::from_node(kind, &built.node),
            Err(_) => {
                self.free_node_ids.push(id);
                return;
            }
        };
        self.node_limit_reached = false;
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

    /// Moves the dragged node `node_id` to `target` (its new top-left corner,
    /// canvas coordinates), kept off negative coordinates. It may pass over other
    /// nodes: overlaps are only resolved when the drag ends.
    ///
    /// Does nothing unless `node_id` is the node being dragged, so the rest of a
    /// cancelled drag gesture is ignored.
    fn drag_to(&mut self, node_id: usize, target: egui::Pos2) {
        if self.dragged_node() != Some(node_id) {
            return;
        }
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            node.pos = target.max(egui::Pos2::ZERO);
        }
    }

    /// Where nodes would go if the drag in progress ended now (`placement::settle`):
    /// `(node id, new position)` for every node that moves, the dragged one
    /// included. Empty when there is no drag or nothing would move. The canvas
    /// paints this as the drop preview.
    fn drop_moves(&self) -> Vec<(usize, egui::Pos2)> {
        let Some(dragged) = self
            .dragged_node()
            .and_then(|id| self.nodes.iter().position(|node| node.id == id))
        else {
            return Vec::new();
        };
        let rects: Vec<_> = self.nodes.iter().map(NodeWidget::rect).collect();
        ui::settle(&rects, dragged)
            .into_iter()
            .zip(&self.nodes)
            .filter(|(pos, node)| *pos != node.pos)
            .map(|(pos, node)| (node.id, pos))
            .collect()
    }

    /// Finishes the drag in progress: the node lands in a free spot nearby or
    /// pushes its neighbors aside ([`drop_moves`](Self::drop_moves)). This is where
    /// the whole drag is known as one move (`from` → final positions), e.g. for
    /// an undo step.
    fn end_drag(&mut self) {
        for (node_id, pos) in self.drop_moves() {
            if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
                node.pos = pos;
            }
        }
        self.drag = None;
    }

    /// Cancels the drag in progress and puts the node back where the drag began.
    /// (That spot is still free: no other node moves before the drag ends.)
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

impl eframe::App for VSignalNodesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::render(self, ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use v_signal_nodes_plugin_std::{GAIN, MIXER, SINK, SOURCE};
    use v_signal_nodes_plugin_stub::STUB_NODE;

    /// The palette entry for node type `type_id`.
    fn kind(app: &VSignalNodesApp, type_id: &str) -> NodeKind {
        app.palette
            .iter()
            .find(|kind| kind.spec().type_id == type_id)
            .cloned()
            .unwrap_or_else(|| panic!("{type_id} is not in the palette"))
    }

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
        let mut app = VSignalNodesApp::default();

        assert!(app.connect(output(0), input(2, 0)));
        assert!(app.connect(output(1), input(2, 0)));

        assert_eq!(app.connections.len(), 1);
        assert_eq!(app.connections[&input(2, 0)], output(1));
    }

    #[test]
    fn connect_refuses_cycles() {
        let mut app = VSignalNodesApp::default();

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
        let mut app = VSignalNodesApp::default();
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
        let mut app = VSignalNodesApp::default();
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
        let mut app = VSignalNodesApp::default();
        for _ in 0..3 {
            app.add_node_at(kind(&app, GAIN), egui::Pos2::ZERO);
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
        app.add_node_at(kind(&app, SINK), egui::Pos2::ZERO);
        assert_eq!(app.nodes.last().unwrap().id, ids[1]);
    }

    #[test]
    fn nodes_added_at_the_same_spot_do_not_overlap() {
        let mut app = VSignalNodesApp::default();
        for _ in 0..5 {
            app.add_node_at(kind(&app, MIXER), egui::pos2(100.0, 100.0));
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
    fn two_nodes() -> VSignalNodesApp {
        let mut app = VSignalNodesApp::default();
        app.add_node_at(kind(&app, GAIN), egui::pos2(0.0, 0.0));
        app.add_node_at(kind(&app, GAIN), egui::pos2(300.0, 0.0));
        app
    }

    fn pos(app: &VSignalNodesApp, node_id: usize) -> egui::Pos2 {
        app.nodes
            .iter()
            .find(|node| node.id == node_id)
            .unwrap()
            .pos
    }

    #[test]
    fn dragging_passes_over_nodes_and_the_drop_lands_next_to_them() {
        let mut app = two_nodes();
        app.begin_drag(0);

        app.drag_to(0, egui::pos2(50.0, 20.0));
        assert_eq!(pos(&app, 0), egui::pos2(50.0, 20.0));

        // Right on top of node 1 while dragging: nothing stops it.
        app.drag_to(0, egui::pos2(310.0, 20.0));
        assert_eq!(pos(&app, 0), egui::pos2(310.0, 20.0));
        assert_eq!(pos(&app, 1), egui::pos2(300.0, 0.0));
        // The preview names what the drop will move: a free spot is close to a lone
        // neighbor, so only the dragged node.
        let moves = app.drop_moves();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].0, 0);

        app.end_drag();
        assert_eq!(app.dragged_node(), None);
        assert_eq!(pos(&app, 0), moves[0].1);
        assert_eq!(pos(&app, 1), egui::pos2(300.0, 0.0));
        assert!(!app.nodes[0].rect().intersects(app.nodes[1].rect()));
        // Outside a drag, moves are ignored and nothing would move.
        let dropped = pos(&app, 0);
        app.drag_to(0, egui::pos2(0.0, 0.0));
        assert_eq!(pos(&app, 0), dropped);
        assert!(app.drop_moves().is_empty());
    }

    #[test]
    fn a_drop_into_a_crowd_stays_and_pushes_the_others_aside() {
        // A column of three gain nodes, a little more than the gap apart, and a
        // fourth node dragged right onto the middle one: no free spot nearby.
        let mut app = VSignalNodesApp::default();
        for _ in 0..4 {
            app.add_node_at(kind(&app, GAIN), egui::pos2(1000.0, 1000.0));
        }
        let step = app.nodes[0].rect().height() + 12.0;
        for (i, node) in app.nodes[..3].iter_mut().enumerate() {
            node.pos = egui::pos2(100.0, 100.0 + i as f32 * step);
        }
        let middle = pos(&app, 1);
        app.begin_drag(3);
        app.drag_to(3, middle);

        app.end_drag();

        assert_eq!(pos(&app, 3), middle, "the dropped node moved");
        assert_ne!(pos(&app, 1), middle, "the node under it didn't make room");
        for (i, a) in app.nodes.iter().enumerate() {
            for b in &app.nodes[i + 1..] {
                assert!(
                    !a.rect().intersects(b.rect()),
                    "{} and {} overlap",
                    a.id,
                    b.id
                );
            }
        }
    }

    #[test]
    fn a_drop_that_barely_overlaps_moves_only_the_dropped_node() {
        let mut app = two_nodes();
        let right_edge = app.nodes[0].rect().right();
        app.begin_drag(1);

        // Just inside the gap next to node 0.
        app.drag_to(1, egui::pos2(right_edge + 2.0, 0.0));
        app.end_drag();

        assert_eq!(pos(&app, 0), egui::pos2(0.0, 0.0));
        // Nudged right just enough to keep the gap, on the same row.
        let gap = pos(&app, 1).x - right_edge;
        assert!(gap > 2.0 && gap < 30.0, "gap {gap}");
        assert_eq!(pos(&app, 1).y, 0.0);
        assert!(!app.nodes[0].rect().intersects(app.nodes[1].rect()));
    }

    #[test]
    fn dragging_never_goes_negative() {
        let mut app = two_nodes();
        app.begin_drag(1);

        app.drag_to(1, egui::pos2(-50.0, -50.0));

        assert!(pos(&app, 1).x >= 0.0 && pos(&app, 1).y >= 0.0);
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
    fn palette_lists_every_plugin_node_in_signal_order() {
        let app = VSignalNodesApp::default();
        let type_ids: Vec<_> = app
            .palette
            .iter()
            .map(|kind| kind.spec().type_id.as_str())
            .collect();

        assert_eq!(type_ids, [SOURCE, GAIN, MIXER, STUB_NODE, SINK]);
    }

    #[test]
    fn nodes_are_built_by_core_and_placed() {
        let mut app = VSignalNodesApp::default();
        let stub = kind(&app, STUB_NODE);

        app.add_node_at(stub.clone(), egui::Pos2::ZERO);

        assert_eq!(app.nodes.len(), 1);
        assert_eq!(app.nodes[0].id, 0);
        // The widget built from the core node has the spec-based preview's size.
        let preview = NodeWidget::new(0, NodeConfig::for_kind(&stub), egui::Pos2::ZERO);
        assert_eq!(app.nodes[0].rect().size(), preview.rect().size());
    }

    #[test]
    fn unknown_plugin_types_are_not_added() {
        let mut app = VSignalNodesApp::default();
        let unknown = v_signal_nodes_core::NodeSpec::new(
            "test.unknown",
            "Unknown",
            v_signal_nodes_core::NodeKind::Generator,
        );

        app.add_node_at(NodeKind::new(unknown), egui::Pos2::ZERO);

        assert!(app.nodes.is_empty());
        assert_eq!(app.free_node_ids.len(), MAX_NODE_COUNT);
    }

    #[test]
    fn raise_node_moves_it_to_the_top() {
        let mut app = VSignalNodesApp::default();
        for _ in 0..3 {
            app.add_node_at(kind(&app, GAIN), egui::Pos2::ZERO);
        }
        let bottom = app.nodes[0].id;

        app.raise_node(bottom);

        assert_eq!(app.nodes.last().unwrap().id, bottom);
        assert_eq!(app.nodes.len(), 3);
    }
}
