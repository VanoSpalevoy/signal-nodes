//! A stub plugin with one node type that does nothing (`stub.thru`).
//!
//! It exists to exercise the whole path a plugin node takes: the spec is
//! registered in `core` through the plugin host, a `core::Node` is built from it,
//! and the editor turns that node into a canvas widget. The processor leaves its
//! output silent.

use v_signal_nodes_plugin_api::{
    AppError, NodeKind, NodeProcessor, NodeProvider, NodeSocket, NodeSpec, ParamSpec, Plugin,
    ProcessIo, Registrar,
};

/// Type id of the stub node.
pub const STUB_NODE: &str = "stub.thru";

/// The stub plugin, named `"stub"`.
pub struct StubPlugin;

impl Plugin for StubPlugin {
    fn name(&self) -> &str {
        "stub"
    }

    fn register(&self, registrar: &mut dyn Registrar) {
        registrar.register_node(Box::new(StubNode));
    }
}

/// [`STUB_NODE`]: one input, one output and one parameter of each kind, so the
/// editor has every kind of label to show.
struct StubNode;

impl NodeProvider for StubNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new(STUB_NODE, "Stub", NodeKind::Processor)
            .input(NodeSocket::audio_in("In"))
            .output(NodeSocket::audio_out("Out"))
            .param(ParamSpec::number("level", "Level", 0.0, 1.0, 0.5))
            .param(ParamSpec::choice("mode", "Mode", ["idle", "busy"], 0))
            .param(ParamSpec::toggle("bypass", "Bypass", true))
            .translate(
                "ru",
                [
                    ("Stub", "Заглушка"),
                    ("In", "Вход"),
                    ("Out", "Выход"),
                    ("Level", "Уровень"),
                    ("Mode", "Режим"),
                    ("idle", "простой"),
                    ("busy", "занят"),
                    ("Bypass", "Обход"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(StubProcessor)
    }
}

/// Does nothing: outputs start zeroed and stay that way.
struct StubProcessor;

impl NodeProcessor for StubProcessor {
    fn process(&mut self, _io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_spec_is_valid() {
        assert!(StubNode.spec().validate().is_ok());
    }
}
