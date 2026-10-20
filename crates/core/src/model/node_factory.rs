//! Building nodes from plugin-provided specs.
//!
//! A plugin describes each node type it offers with a [`NodeSpec`]. Specs are
//! registered in a [`NodeFactory`], validated once, and then used to stamp out
//! [`Node`] instances. The spec lives in `core` (not in `plugin-api`) so both the
//! plugin API and the host can use it without a dependency cycle.

use std::collections::BTreeMap;

use crate::AppError;
use crate::model::{Node, NodeKind, NodeSocket, ParamSpec, SocketDirection};

/// The description of one node type, as provided by a plugin.
///
/// All labels (`name`, socket names, parameter names and choice options) are the
/// English defaults; [`translations`](Self::translations) can provide them in
/// other languages.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSpec {
    /// Globally unique type id, namespaced by the plugin, e.g. `"std.gain"`.
    pub type_id: String,
    /// Display name, copied to every built node.
    pub name: String,
    pub kind: NodeKind,
    /// Input sockets; each must have [`SocketDirection::Input`].
    pub inputs: Vec<NodeSocket>,
    /// Output sockets; each must have [`SocketDirection::Output`].
    pub outputs: Vec<NodeSocket>,
    /// Parameters; ids must be unique.
    pub params: Vec<ParamSpec>,
    /// Translations of the labels above.
    pub translations: Vec<Translation>,
}

/// A label of a [`NodeSpec`] in another language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translation {
    /// Language code, e.g. `"ru"`.
    pub language: String,
    /// The English label as written in the spec.
    pub source: String,
    /// The label in `language`.
    pub text: String,
}

impl NodeSpec {
    /// A spec with no sockets, parameters or translations; add them with the
    /// builder methods ([`input`](Self::input), [`param`](Self::param), ...).
    pub fn new(type_id: impl Into<String>, name: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            type_id: type_id.into(),
            name: name.into(),
            kind,
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: Vec::new(),
            translations: Vec::new(),
        }
    }

    /// Adds a parameter.
    pub fn param(mut self, param: ParamSpec) -> Self {
        self.params.push(param);
        self
    }

    /// Adds translations of labels into `language`, as `(source, text)` pairs.
    pub fn translate<'a>(
        mut self,
        language: &str,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        self.translations
            .extend(pairs.into_iter().map(|(source, text)| Translation {
                language: language.to_owned(),
                source: source.to_owned(),
                text: text.to_owned(),
            }));
        self
    }

    /// `label` in `language`, or `label` itself when there's no translation.
    pub fn label<'a>(&'a self, language: &str, label: &'a str) -> &'a str {
        self.translations
            .iter()
            .find(|translation| translation.language == language && translation.source == label)
            .map_or(label, |translation| &translation.text)
    }

    /// Adds an input socket.
    pub fn input(mut self, socket: NodeSocket) -> Self {
        self.inputs.push(socket);
        self
    }

    /// Adds an output socket.
    pub fn output(mut self, socket: NodeSocket) -> Self {
        self.outputs.push(socket);
        self
    }

    /// Checks the spec is usable: non-empty type id and name, sockets in the right
    /// list, socket names unique per direction, sockets that fit the kind
    /// (generators have no inputs, outputs have no outputs, processors need both),
    /// and valid parameters with unique ids.
    pub fn validate(&self) -> Result<(), AppError> {
        let invalid = |message: String| {
            Err(AppError::InvalidInput(format!(
                "node spec '{}': {message}",
                self.type_id
            )))
        };

        if self.type_id.trim().is_empty() {
            return invalid("empty type id".into());
        }
        if self.name.trim().is_empty() {
            return invalid("empty name".into());
        }

        for (sockets, direction) in [
            (&self.inputs, SocketDirection::Input),
            (&self.outputs, SocketDirection::Output),
        ] {
            for (index, socket) in sockets.iter().enumerate() {
                if socket.direction != direction {
                    return invalid(format!(
                        "socket '{}' listed as {direction:?} but is {:?}",
                        socket.name, socket.direction
                    ));
                }
                if sockets[..index]
                    .iter()
                    .any(|other| other.name == socket.name)
                {
                    return invalid(format!("duplicate {direction:?} socket '{}'", socket.name));
                }
            }
        }

        let (has_inputs, has_outputs) = (!self.inputs.is_empty(), !self.outputs.is_empty());
        let fits_kind = match self.kind {
            NodeKind::Generator => !has_inputs && has_outputs,
            NodeKind::Processor => has_inputs && has_outputs,
            NodeKind::Output => has_inputs && !has_outputs,
        };
        if !fits_kind {
            return invalid(format!(
                "{:?} node with {} input(s) and {} output(s)",
                self.kind,
                self.inputs.len(),
                self.outputs.len()
            ));
        }

        for (index, param) in self.params.iter().enumerate() {
            if let Err(err) = param.validate() {
                return invalid(err.to_string());
            }
            if self.params[..index]
                .iter()
                .any(|other| other.id == param.id)
            {
                return invalid(format!("duplicate parameter '{}'", param.id));
            }
        }

        Ok(())
    }
}

/// Registry of node specs that builds [`Node`] instances.
///
/// Node ids are chosen by the caller (the graph owns id allocation), the factory
/// only fills in everything that comes from the spec.
#[derive(Debug, Default)]
pub struct NodeFactory {
    /// Registered specs by type id (sorted, so listings are stable).
    specs: BTreeMap<String, NodeSpec>,
}

impl NodeFactory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates and registers `spec`. Fails if the spec is invalid or its type id
    /// is already taken.
    pub fn register(&mut self, spec: NodeSpec) -> Result<(), AppError> {
        spec.validate()?;
        if self.specs.contains_key(&spec.type_id) {
            return Err(AppError::InvalidInput(format!(
                "node type '{}' is already registered",
                spec.type_id
            )));
        }
        self.specs.insert(spec.type_id.clone(), spec);
        Ok(())
    }

    /// The spec registered under `type_id`.
    pub fn spec(&self, type_id: &str) -> Option<&NodeSpec> {
        self.specs.get(type_id)
    }

    /// All registered specs, sorted by type id.
    pub fn specs(&self) -> impl Iterator<Item = &NodeSpec> {
        self.specs.values()
    }

    /// Builds node `id` of type `type_id`, with every parameter at its default.
    pub fn build_node(&self, type_id: &str, id: usize) -> Result<Node, AppError> {
        let spec = self
            .specs
            .get(type_id)
            .ok_or_else(|| AppError::InvalidInput(format!("unknown node type '{type_id}'")))?;
        Ok(Node {
            id,
            type_id: spec.type_id.clone(),
            name: spec.name.clone(),
            kind: spec.kind,
            inputs: spec.inputs.clone(),
            outputs: spec.outputs.clone(),
            params: spec.params.iter().map(ParamSpec::default_value).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ParamValue;

    fn gain() -> NodeSpec {
        NodeSpec::new("test.gain", "Gain", NodeKind::Processor)
            .input(NodeSocket::audio_in("in"))
            .output(NodeSocket::audio_out("out"))
            .param(ParamSpec::number("gain", "Gain", 0.0, 2.0, 1.0))
            .translate("ru", [("Gain", "Усиление")])
    }

    #[test]
    fn builds_nodes_from_specs() {
        let mut factory = NodeFactory::new();
        factory.register(gain()).unwrap();

        let node = factory.build_node("test.gain", 7).unwrap();

        assert_eq!(node.id, 7);
        assert_eq!(node.type_id, "test.gain");
        assert_eq!(node.name, "Gain");
        assert_eq!(node.kind, NodeKind::Processor);
        assert_eq!(node.inputs, gain().inputs);
        assert_eq!(node.outputs, gain().outputs);
        assert_eq!(node.params, [ParamValue::Number(1.0)]);
    }

    #[test]
    fn labels_fall_back_to_english() {
        let spec = gain();

        assert_eq!(spec.label("ru", "Gain"), "Усиление");
        assert_eq!(spec.label("ru", "in"), "in");
        assert_eq!(spec.label("en", "Gain"), "Gain");
    }

    #[test]
    fn rejects_duplicate_and_unknown_types() {
        let mut factory = NodeFactory::new();
        factory.register(gain()).unwrap();

        assert!(factory.register(gain()).is_err());
        assert!(factory.build_node("test.missing", 0).is_err());
    }

    #[test]
    fn rejects_invalid_specs() {
        let wrong_direction = NodeSpec::new("t.a", "A", NodeKind::Processor)
            .input(NodeSocket::audio_out("in"))
            .output(NodeSocket::audio_out("out"));
        let duplicate_socket = gain().input(NodeSocket::audio_in("in"));
        let generator_with_input =
            NodeSpec::new("t.b", "B", NodeKind::Generator).input(NodeSocket::audio_in("in"));
        let empty_id = NodeSpec {
            type_id: " ".into(),
            ..gain()
        };
        let duplicate_param = gain().param(ParamSpec::toggle("gain", "On", true));
        let invalid_param = gain().param(ParamSpec::choice("mode", "Mode", ["a"], 3));

        for spec in [
            wrong_direction,
            duplicate_socket,
            generator_with_input,
            empty_id,
            duplicate_param,
            invalid_param,
        ] {
            assert!(spec.validate().is_err(), "{spec:?}");
        }
        assert!(gain().validate().is_ok());
    }
}
