//! Building nodes from plugin-provided specs.
//!
//! A plugin describes each node type it offers with a [`NodeSpec`]. Specs are
//! registered in a [`NodeFactory`], validated once, and then used to stamp out
//! [`Node`] instances. The spec lives in `core` (not in `plugin-api`) so both the
//! plugin API and the host can use it without a dependency cycle.

use std::collections::BTreeMap;

use crate::AppError;
use crate::model::{Node, NodeKind, NodeSocket, ParamKind, ParamSpec, SocketDirection};

/// The description of one node type, as provided by a plugin.
///
/// All labels (`name`, socket names, parameter names and choice options) are the
/// English defaults; [`translations`](Self::translations) can provide them in
/// other languages, addressed by [`TextKey`].
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

/// Which label of a [`NodeSpec`] a [`Translation`] is for.
///
/// Labels are addressed by their role and position or id, not by their English
/// text, so editing the English label keeps its translations, and the same English
/// word in two roles (a node and a parameter named "Gain") can be translated
/// differently. Sockets and choice options are addressed by index, which is
/// already their identity: connections and parameter values refer to them that way.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextKey {
    /// The node type's [`name`](NodeSpec::name).
    Name,
    /// Name of input socket `index`.
    Input(usize),
    /// Name of output socket `index`.
    Output(usize),
    /// Name of the parameter with this [`id`](ParamSpec::id).
    Param(String),
    /// Option `index` of the choice parameter with this id.
    Option(String, usize),
}

impl TextKey {
    /// [`Param`](Self::Param) `id`.
    pub fn param(id: impl Into<String>) -> Self {
        Self::Param(id.into())
    }

    /// [`Option`](Self::Option) `index` of parameter `id`.
    pub fn option(id: impl Into<String>, index: usize) -> Self {
        Self::Option(id.into(), index)
    }
}

/// A label of a [`NodeSpec`] in another language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translation {
    /// Language code, e.g. `"ru"`.
    pub language: String,
    /// The label being translated.
    pub key: TextKey,
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

    /// Adds translations of labels into `language`, as `(key, text)` pairs.
    pub fn translate<'a>(
        mut self,
        language: &str,
        pairs: impl IntoIterator<Item = (TextKey, &'a str)>,
    ) -> Self {
        self.translations
            .extend(pairs.into_iter().map(|(key, text)| Translation {
                language: language.to_owned(),
                key,
                text: text.to_owned(),
            }));
        self
    }

    /// The English label `key` refers to, or `None` if there's no such label.
    pub fn english(&self, key: &TextKey) -> Option<&str> {
        let param = |id: &str| self.params.iter().find(|param| param.id == id);
        match key {
            TextKey::Name => Some(&self.name),
            TextKey::Input(index) => self.inputs.get(*index).map(|socket| socket.name.as_str()),
            TextKey::Output(index) => self.outputs.get(*index).map(|socket| socket.name.as_str()),
            TextKey::Param(id) => param(id).map(|param| param.name.as_str()),
            TextKey::Option(id, index) => match &param(id)?.kind {
                ParamKind::Choice { options, .. } => options.get(*index).map(String::as_str),
                _ => None,
            },
        }
    }

    /// Label `key` in `language`, or in English when there's no translation; empty
    /// if the spec has no such label.
    pub fn label(&self, language: &str, key: &TextKey) -> &str {
        self.translations
            .iter()
            .find(|translation| translation.language == language && translation.key == *key)
            .map(|translation| translation.text.as_str())
            .or_else(|| self.english(key))
            .unwrap_or_default()
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
    /// valid parameters with unique ids, and translations of existing labels, at
    /// most one per language and label.
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

        for (index, translation) in self.translations.iter().enumerate() {
            if self.english(&translation.key).is_none() {
                return invalid(format!(
                    "translation of a missing label {:?}",
                    translation.key
                ));
            }
            if self.translations[..index]
                .iter()
                .any(|other| other.language == translation.language && other.key == translation.key)
            {
                return invalid(format!(
                    "duplicate '{}' translation of {:?}",
                    translation.language, translation.key
                ));
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
            .param(ParamSpec::choice("mode", "Mode", ["soft", "hard"], 0))
            .translate(
                "ru",
                [
                    (TextKey::Name, "Усилитель"),
                    (TextKey::param("gain"), "Усиление"),
                    (TextKey::option("mode", 1), "жёсткий"),
                ],
            )
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
        assert_eq!(
            node.params,
            [ParamValue::Number(1.0), ParamValue::Choice(0)]
        );
    }

    #[test]
    fn labels_are_keyed_by_role_and_fall_back_to_english() {
        let spec = gain();

        // Same English text, different roles, different translations.
        assert_eq!(spec.label("ru", &TextKey::Name), "Усилитель");
        assert_eq!(spec.label("ru", &TextKey::param("gain")), "Усиление");
        assert_eq!(spec.label("ru", &TextKey::option("mode", 1)), "жёсткий");
        assert_eq!(spec.label("ru", &TextKey::option("mode", 0)), "soft");
        assert_eq!(spec.label("ru", &TextKey::Input(0)), "in");
        assert_eq!(spec.label("en", &TextKey::Name), "Gain");
        assert_eq!(spec.label("ru", &TextKey::Output(5)), "");

        // Renaming the English label keeps the translation.
        let renamed = NodeSpec {
            name: "Amplifier".into(),
            ..gain()
        };
        assert_eq!(renamed.label("ru", &TextKey::Name), "Усилитель");
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
        let invalid_param = gain().param(ParamSpec::choice("pan", "Pan", ["a"], 3));
        let missing_label = gain().translate("ru", [(TextKey::option("gain", 0), "x")]);
        let duplicate_translation = gain().translate("ru", [(TextKey::Name, "Другое")]);

        for spec in [
            wrong_direction,
            duplicate_socket,
            generator_with_input,
            empty_id,
            duplicate_param,
            invalid_param,
            missing_label,
            duplicate_translation,
        ] {
            assert!(spec.validate().is_err(), "{spec:?}");
        }
        assert!(gain().validate().is_ok());
    }
}
