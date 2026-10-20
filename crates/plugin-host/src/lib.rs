//! Loads plugins and builds nodes from the node types they provide.
//!
//! [`PluginHost::load`] collects a plugin's [`NodeProvider`]s, registers their
//! specs in a [`NodeFactory`] and keeps the providers by type id.
//! [`PluginHost::build_node`] then returns the [`Node`] together with its
//! [`NodeProcessor`].
//!
//! Plugins built as a `cdylib` with `export_plugin!` are loaded with
//! [`PluginHost::load_library`] (see [`DynamicPlugin`]).

use std::collections::HashMap;
use std::ffi::OsStr;

use signal_nodes_core::{AppError, Node, NodeFactory, NodeSpec};
use signal_nodes_plugin_api::{API_VERSION, NodeProcessor, NodeProvider, Plugin, Registrar};

mod dynamic;

pub use dynamic::DynamicPlugin;

/// A node built from a plugin: the graph-side description plus its DSP state.
pub struct PluginNode {
    pub node: Node,
    pub processor: Box<dyn NodeProcessor>,
}

/// Registry of loaded plugins and their node types.
#[derive(Default)]
pub struct PluginHost {
    factory: NodeFactory,
    /// Providers by node type id.
    providers: HashMap<String, Box<dyn NodeProvider>>,
    /// Names of loaded plugins, in load order.
    plugins: Vec<String>,
}

/// [`Registrar`] that only collects providers, so a plugin is registered all or
/// nothing.
#[derive(Default)]
struct Collector(Vec<Box<dyn NodeProvider>>);

impl Registrar for Collector {
    fn register_node(&mut self, provider: Box<dyn NodeProvider>) {
        self.0.push(provider);
    }
}

impl PluginHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `plugin` and registers all its node types. Fails without changing
    /// anything if the API version doesn't match, the plugin is already loaded, or
    /// any of its node specs is invalid or clashes with a registered type.
    pub fn load(&mut self, plugin: &dyn Plugin) -> Result<(), AppError> {
        let name = plugin.name();
        if plugin.api_version() != API_VERSION {
            return Err(AppError::InvalidInput(format!(
                "plugin '{name}' targets API v{}, host is v{API_VERSION}",
                plugin.api_version()
            )));
        }
        if self.plugins.iter().any(|loaded| loaded == name) {
            return Err(AppError::InvalidInput(format!(
                "plugin '{name}' is already loaded"
            )));
        }

        let mut collector = Collector::default();
        plugin.register(&mut collector);

        // Validate everything on a scratch factory first, so a bad plugin leaves
        // no partial registration behind.
        let mut staged = NodeFactory::new();
        let mut entries = Vec::with_capacity(collector.0.len());
        for provider in collector.0 {
            let spec = provider.spec();
            if self.factory.spec(&spec.type_id).is_some() {
                return Err(AppError::InvalidInput(format!(
                    "plugin '{name}': node type '{}' is already registered",
                    spec.type_id
                )));
            }
            staged.register(spec.clone())?;
            entries.push((spec, provider));
        }

        for (spec, provider) in entries {
            self.providers.insert(spec.type_id.clone(), provider);
            self.factory.register(spec)?;
        }
        self.plugins.push(name.to_owned());
        Ok(())
    }

    /// Loads the plugin library at `path` (a `cdylib` built with
    /// `export_plugin!`) and registers its node types like [`load`](Self::load).
    /// The library stays loaded while any of its providers or processors exist.
    ///
    /// # Safety
    ///
    /// Same as [`DynamicPlugin::open`]: the library must be a trusted plugin.
    pub unsafe fn load_library(&mut self, path: impl AsRef<OsStr>) -> Result<(), AppError> {
        // SAFETY: forwarded to the caller.
        let plugin = unsafe { DynamicPlugin::open(path) }?;
        self.load(&plugin)
    }

    /// Names of the loaded plugins, in load order.
    pub fn plugins(&self) -> &[String] {
        &self.plugins
    }

    /// All available node types, sorted by type id.
    pub fn specs(&self) -> impl Iterator<Item = &NodeSpec> {
        self.factory.specs()
    }

    /// Builds node `id` of type `type_id` with a fresh processor.
    pub fn build_node(&self, type_id: &str, id: usize) -> Result<PluginNode, AppError> {
        let node = self.factory.build_node(type_id, id)?;
        let processor = self.providers[type_id].create_processor();
        Ok(PluginNode { node, processor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signal_nodes_core::{NodeKind, NodeSocket};
    use signal_nodes_plugin_api::{ProcessIo, RawBuffer, ffi};

    struct Gain;

    struct GainProcessor {
        factor: f32,
    }

    impl NodeProcessor for GainProcessor {
        fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError> {
            let input = io.input(0);
            for (out, sample) in io.output(0).iter_mut().zip(input) {
                *out = sample * self.factor;
            }
            Ok(())
        }
    }

    impl NodeProvider for Gain {
        fn spec(&self) -> NodeSpec {
            NodeSpec::new("test.gain", "Gain", NodeKind::Processor)
                .input(NodeSocket::audio_in("in"))
                .output(NodeSocket::audio_out("out"))
        }

        fn create_processor(&self) -> Box<dyn NodeProcessor> {
            Box::new(GainProcessor { factor: 2.0 })
        }
    }

    struct TestPlugin;

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            "test"
        }

        fn register(&self, registrar: &mut dyn Registrar) {
            registrar.register_node(Box::new(Gain));
        }
    }

    /// Runs a one-input, one-output processor over `input`.
    fn run(processor: &mut dyn NodeProcessor, input: &[f32]) -> Vec<f32> {
        let mut output = vec![0.0; input.len()];
        let inputs = [RawBuffer::input(input)];
        let outputs = [RawBuffer::output(&mut output)];
        // SAFETY: both buffers are `input.len()` long, live for the call and
        // don't overlap.
        let mut io = unsafe { ProcessIo::from_raw(48_000.0, input.len(), &[], &inputs, &outputs) };
        processor.process(&mut io).unwrap();
        output
    }

    #[test]
    fn builds_a_working_node_from_a_plugin() {
        let mut host = PluginHost::new();
        host.load(&TestPlugin).unwrap();

        let mut built = host.build_node("test.gain", 3).unwrap();

        assert_eq!(built.node.id, 3);
        assert_eq!(built.node.type_id, "test.gain");
        assert_eq!(run(built.processor.as_mut(), &[1.0, -0.5]), [2.0, -1.0]);
        assert_eq!(host.plugins(), ["test"]);
    }

    #[test]
    fn builds_a_working_node_through_the_c_abi() {
        // SAFETY: `export` produces a plugin object following the `ffi` protocol,
        // and its code is part of this binary.
        let plugin = unsafe { DynamicPlugin::from_ffi(ffi::export(|| TestPlugin), None) }.unwrap();
        let mut host = PluginHost::new();
        host.load(&plugin).unwrap();
        drop(plugin);

        let mut built = host.build_node("test.gain", 0).unwrap();

        assert_eq!(run(built.processor.as_mut(), &[1.0, -0.5]), [2.0, -1.0]);
        assert_eq!(host.plugins(), ["test"]);
    }

    #[test]
    fn rejects_a_missing_library() {
        let mut host = PluginHost::new();

        // SAFETY: the path doesn't exist, so nothing is loaded.
        assert!(unsafe { host.load_library("no-such-plugin-library") }.is_err());
        assert!(host.plugins().is_empty());
    }

    #[test]
    fn rejects_loading_twice_and_unknown_types() {
        let mut host = PluginHost::new();
        host.load(&TestPlugin).unwrap();

        assert!(host.load(&TestPlugin).is_err());
        assert!(host.build_node("test.missing", 0).is_err());
    }
}
