//! Editor-side node model and geometry.
//!
//! This is what the canvas draws and edits. It is separate from the processing
//! model in `signal-forge-core`, which the editor doesn't use yet.
//!
//! All positions here are **canvas coordinates**: `(0, 0)` is the top-left of the
//! scrollable canvas content, independent of scrolling and window layout. The
//! canvas converts them to screen coordinates when drawing.
//!
//! # Node layout
//!
//! ```text
//!   pos
//!    ┌──────────────────────────────┐ ─┬─
//!    │ [x]                  Gain 3  │  │ CONTENT_TOP: delete button and title
//!    │                              │ ─┴─
//!    ● Audio in Gain: 1.0 Audio out ●    row 0, ROW_HEIGHT tall
//!    │                              │
//!    └──────────────────────────────┘
//!    ◄───────── NODE_WIDTH ─────────►
//! ```
//!
//! Row `i` holds input `i` on the left edge, output `i` on the right edge and
//! parameter `i` in the middle. The node is as tall as its longest column, but at
//! least [`MIN_NODE_HEIGHT`].

use eframe::egui;

use super::super::localization::{Language, Text};

/// Width of every node.
const NODE_WIDTH: f32 = 160.0;
/// Height of a node with a single row.
const MIN_NODE_HEIGHT: f32 = 90.0;
/// Height of one socket/parameter row.
const ROW_HEIGHT: f32 = 22.0;
/// Height of the header above the first row.
const CONTENT_TOP: f32 = 38.0;

/// Type of a node; decides its sockets, parameters and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Source,
    Gain,
    Mixer,
    Sink,
}

impl NodeKind {
    /// Display name, used for the palette button and the node title.
    pub(super) fn label(self, language: Language) -> &'static str {
        match self {
            Self::Source => language.text(Text::Source),
            Self::Gain => language.text(Text::Gain),
            Self::Mixer => language.text(Text::Mixer),
            Self::Sink => language.text(Text::Sink),
        }
    }
}

/// Side of a node a socket is on. Inputs are on the left edge, outputs on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketKind {
    Input,
    Output,
}

impl SocketKind {
    /// Display name ("Input"/"Output"), used in the debug panel.
    pub(super) fn label(self, language: Language) -> &'static str {
        match self {
            Self::Input => language.text(Text::Input),
            Self::Output => language.text(Text::Output),
        }
    }
}

/// Address of one socket: node, side and row.
///
/// Used as both key and value of `SignalForgeApp::connections`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SocketRef {
    /// ID of the node ([`NodeWidget::id`]).
    pub(super) node_id: usize,
    pub(super) side: SocketKind,
    /// 0-based row among the sockets on that side; shown 1-based in the UI.
    pub(super) index: usize,
}

/// Value of a node parameter.
#[derive(Clone, Copy, Debug)]
pub enum ParamValue {
    /// A word that is translated with the UI, such as "stereo".
    Text(Text),
    /// A number, shown with one decimal place.
    Number(f32),
}

/// A named node parameter, shown as `Name: value` in the middle of the node.
/// Display only for now: parameters can't be edited yet.
#[derive(Clone, Copy, Debug)]
pub struct NodeParam {
    name: Text,
    value: ParamValue,
}

impl NodeParam {
    /// `Name: value` in `language`, for example "Gain: 1.0" or "Режим: стерео".
    pub(super) fn label(self, language: Language) -> String {
        let name = language.text(self.name);
        match self.value {
            ParamValue::Text(value) => format!("{name}: {}", language.text(value)),
            ParamValue::Number(value) => format!("{name}: {value:.1}"),
        }
    }
}

/// What a node contains: its kind, socket labels and parameters.
///
/// Holds translation keys, not strings, so nodes follow language changes.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub(super) kind: NodeKind,
    /// Input socket labels, top to bottom; the position is the socket index.
    pub(super) inputs: Vec<Text>,
    /// Output socket labels, top to bottom; the position is the socket index.
    pub(super) outputs: Vec<Text>,
    /// Parameters, top to bottom.
    pub(super) content: Vec<NodeParam>,
}

impl NodeConfig {
    /// Default sockets and parameters for a new node of `kind`.
    ///
    /// | Kind   | Inputs              | Outputs   | Parameters                    |
    /// |--------|---------------------|-----------|-------------------------------|
    /// | Source | —                   | Audio     | Signal: sine                  |
    /// | Gain   | Audio in            | Audio out | Gain: 1.0                     |
    /// | Mixer  | Left, Right, Side   | Mixed     | Mode: stereo, Gain, Pan       |
    /// | Sink   | Audio               | —         | Channels: stereo              |
    pub(in crate::editor) fn for_kind(kind: NodeKind) -> Self {
        let param = |name, value| NodeParam { name, value };
        let (inputs, outputs, content) = match kind {
            NodeKind::Source => (
                vec![],
                vec![Text::Audio],
                vec![param(Text::Signal, ParamValue::Text(Text::Sine))],
            ),
            NodeKind::Gain => (
                vec![Text::AudioIn],
                vec![Text::AudioOut],
                vec![param(Text::Gain, ParamValue::Number(1.0))],
            ),
            NodeKind::Mixer => (
                vec![Text::Left, Text::Right, Text::Side],
                vec![Text::Mixed],
                vec![
                    param(Text::Mode, ParamValue::Text(Text::Stereo)),
                    param(Text::Gain, ParamValue::Number(1.0)),
                    param(Text::Pan, ParamValue::Text(Text::Center)),
                ],
            ),
            NodeKind::Sink => (
                vec![Text::Audio],
                vec![],
                vec![param(Text::Channels, ParamValue::Text(Text::Stereo))],
            ),
        };

        Self {
            kind,
            inputs,
            outputs,
            content,
        }
    }

    /// Size of a node with this config: [`NODE_WIDTH`] wide, tall enough for the
    /// longest of the inputs, outputs and parameters.
    ///
    /// The single source of node size: the node itself, the drop preview and the
    /// canvas extent all use it.
    pub(super) fn size(&self) -> egui::Vec2 {
        let row_count = self
            .inputs
            .len()
            .max(self.outputs.len())
            .max(self.content.len())
            .max(1);
        egui::vec2(
            NODE_WIDTH,
            MIN_NODE_HEIGHT.max(CONTENT_TOP + ROW_HEIGHT * row_count as f32 + 8.0),
        )
    }
}

/// A node placed on the canvas.
#[derive(Clone, Debug)]
pub struct NodeWidget {
    /// Slot from `SignalForgeApp::free_node_ids`; unique among live nodes and
    /// reused after the node is deleted. Shown in the node title.
    pub(super) id: usize,
    pub(super) config: NodeConfig,
    /// Top-left corner in canvas coordinates; kept `>= 0` by [`Self::clamp_to_canvas`].
    pub(super) pos: egui::Pos2,
}

impl NodeWidget {
    pub(in crate::editor) fn new(id: usize, config: NodeConfig, pos: egui::Pos2) -> Self {
        Self { id, config, pos }
    }

    /// The node's bounds in canvas coordinates.
    pub(super) fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.pos, self.config.size())
    }

    /// Keeps the node off negative coordinates.
    ///
    /// The canvas grows right and down, so only the top-left edges limit a node.
    /// Called after user moves only, never per frame, so resizing the window
    /// doesn't move nodes.
    pub(super) fn clamp_to_canvas(&mut self) {
        self.pos = self.pos.max(egui::Pos2::ZERO);
    }

    /// Socket labels on `side`, top to bottom.
    pub(super) fn sockets(&self, side: SocketKind) -> &[Text] {
        match side {
            SocketKind::Input => &self.config.inputs,
            SocketKind::Output => &self.config.outputs,
        }
    }

    /// Center of a socket in canvas coordinates: on the left (input) or right
    /// (output) edge, vertically centered in row `index`.
    ///
    /// An out-of-range `index` falls back to row 0.
    pub(super) fn socket_pos(&self, side: SocketKind, index: usize) -> egui::Pos2 {
        let sockets = self.sockets(side);
        let row = if index < sockets.len() { index } else { 0 };
        egui::pos2(
            match side {
                SocketKind::Input => self.pos.x,
                SocketKind::Output => self.pos.x + NODE_WIDTH,
            },
            self.pos.y + CONTENT_TOP + ROW_HEIGHT * (row as f32 + 0.5),
        )
        .clamp(self.rect().min, self.rect().max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_geometry_expands_for_custom_sockets_and_content() {
        let config = NodeConfig::for_kind(NodeKind::Mixer);
        let node = NodeWidget::new(0, config, egui::Pos2::ZERO);

        assert!(node.rect().height() > MIN_NODE_HEIGHT);
        assert_eq!(node.config.inputs.len(), 3);
        assert_eq!(node.config.outputs.len(), 1);
        assert_ne!(
            node.socket_pos(SocketKind::Input, 0),
            node.socket_pos(SocketKind::Input, 2),
        );
    }

    #[test]
    fn node_param_label_follows_language() {
        let config = NodeConfig::for_kind(NodeKind::Gain);
        let param = config.content[0];

        assert_eq!(param.label(Language::English), "Gain: 1.0");
        assert_eq!(param.label(Language::Russian), "Усиление: 1.0");
    }

    #[test]
    fn clamp_limits_only_top_left() {
        let mut node = NodeWidget::new(
            0,
            NodeConfig::for_kind(NodeKind::Mixer),
            egui::pos2(-20.0, -5.0),
        );

        node.clamp_to_canvas();
        assert_eq!(node.pos, egui::Pos2::ZERO);

        node.pos = egui::pos2(10_000.0, 10_000.0);
        node.clamp_to_canvas();
        assert_eq!(node.pos, egui::pos2(10_000.0, 10_000.0));
    }
}
