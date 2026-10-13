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
use ui::{NodeConfig, NodeKind, NodeWidget, SocketKind, SocketRef};

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
///   and snapping through `ui::placement`.
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
    /// connection or source selection.
    connection_refused: bool,
    /// Node kind clicked in the palette this frame; the canvas adds it in the
    /// middle of the visible area and clears it. (Palette drags don't use this:
    /// they travel as an egui drag-and-drop payload.)
    requested_node: Option<NodeKind>,
    /// UI language; every visible string is resolved through it on each frame.
    language: Language,
    /// Canvas zoom chosen in the toolbar: 1.0 = one canvas unit per screen point.
    scale: f32,
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
