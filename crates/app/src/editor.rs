//! Editor state and the `eframe` application entry.
//!
//! [`SignalForgeApp`] owns the whole graph being edited (nodes and connections) and
//! the transient UI state (socket selection, palette drag, language). Rendering and
//! input handling live in the [`ui`] module, which mutates this state directly.

use eframe::egui;
use std::collections::HashMap;

mod localization;
mod ui;

use localization::{Language, Text};
use ui::NodeConfig;
pub(super) use ui::{NodeKind, NodeWidget, SocketKind, SocketRef};

/// Maximum number of nodes on the canvas; also the size of the node ID pool.
pub(super) const MAX_NODE_COUNT: usize = 256;

/// The node editor.
///
/// Invariants that every add/remove path must keep:
/// - each ID in `0..MAX_NODE_COUNT` is either used by exactly one node in `nodes`
///   or sits in `free_node_ids`, never both;
/// - `connections` and `pending_source` only refer to nodes that exist;
/// - `node_limit_reached` is set only by a failed [`add_node_at`](Self::add_node_at)
///   and cleared by the next successful add or any deletion.
pub struct SignalForgeApp {
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
    /// Node kind being dragged from the palette; dropped onto the canvas on release.
    dragged_palette_kind: Option<NodeKind>,
    /// UI language; every visible string is resolved through it on each frame.
    language: Language,
    /// Canvas zoom chosen in the toolbar. Not applied to rendering yet.
    scale: f32,
}

impl Default for SignalForgeApp {
    fn default() -> Self {
        let app = Self {
            nodes: Vec::new(),
            connections: HashMap::new(),
            free_node_ids: (0..MAX_NODE_COUNT).rev().collect(),
            node_limit_reached: false,
            pending_source: None,
            dragged_palette_kind: None,
            language: Language::English,
            scale: 1.0,
        };

        app
    }
}

impl SignalForgeApp {
    /// Adds a node of `kind` with its top-left corner at `pos` (canvas coordinates).
    ///
    /// When all [`MAX_NODE_COUNT`] IDs are in use, nothing is added and
    /// `node_limit_reached` is set instead.
    fn add_node_at(&mut self, kind: NodeKind, pos: egui::Pos2) {
        let Some(id) = self.free_node_ids.pop() else {
            self.node_limit_reached = true;
            return;
        };
        self.node_limit_reached = false;
        self.nodes
            .push(NodeWidget::new(id, NodeConfig::for_kind(kind), pos));
    }

    /// Draws the whole editor for one frame and applies this frame's input.
    fn render(&mut self, ui: &mut egui::Ui) {
        ui::render(self, ui);
    }
}

impl eframe::App for SignalForgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render(ui);
    }
}
