//! Plugins loaded from dynamic libraries, wrapped back into the plugin traits.
//!
//! [`DynamicPlugin`] reads the library's [`FfiPlugin`] once (name and specs are
//! copied), then hands out [`NodeProvider`]s and [`NodeProcessor`]s that call
//! through the C ABI. Every wrapper holds an `Arc` of the loaded library, so the
//! library stays loaded until the last provider and processor are dropped, and
//! the plugin's own `drop` runs after all its processors are gone.

use std::ffi::OsStr;
use std::sync::Arc;

use signal_nodes_core::{AppError, NodeSpec};
use signal_nodes_plugin_api::ffi::{
    self, EntryFn, FfiPlugin, FfiProcessArgs, FfiProcessor, FfiSlice, VersionFn,
};
use signal_nodes_plugin_api::{
    API_VERSION, NodeProcessor, NodeProvider, Plugin, ProcessIo, Registrar,
};

/// A plugin loaded from a dynamic library.
pub struct DynamicPlugin {
    loaded: Arc<Loaded>,
    name: String,
    specs: Vec<NodeSpec>,
}

/// The plugin object together with the library its code lives in.
struct Loaded {
    plugin: FfiPlugin,
    /// Kept loaded until `plugin` is dropped (fields drop after `Drop::drop`).
    _library: Option<libloading::Library>,
}

// SAFETY: the plugin side is built from `NodeProvider`s, which are `Send + Sync`,
// and only reads its state through `ctx`.
unsafe impl Send for Loaded {}
// SAFETY: as above.
unsafe impl Sync for Loaded {}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: `ctx` is non-null (checked in `from_ffi`) and dropped only here.
        unsafe { (self.plugin.drop)(self.plugin.ctx) };
    }
}

impl DynamicPlugin {
    /// Loads the plugin library at `path`.
    ///
    /// # Safety
    ///
    /// Loading a library runs its initialization code, and the plugin's code then
    /// runs in this process: the library must be a trusted plugin built with
    /// `export_plugin!`.
    pub unsafe fn open(path: impl AsRef<OsStr>) -> Result<Self, AppError> {
        let path = path.as_ref();
        let error = |message: String| AppError::Plugin(format!("{}: {message}", path.display()));
        // SAFETY: forwarded to the caller.
        unsafe {
            let library = libloading::Library::new(path).map_err(|err| error(err.to_string()))?;
            let version = library
                .get::<VersionFn>(ffi::VERSION_SYMBOL)
                .map_err(|err| error(format!("not a plugin ({err})")))?;
            let version = version();
            if version != API_VERSION {
                return Err(error(format!(
                    "built for plugin API v{version}, host is v{API_VERSION}"
                )));
            }
            let entry = library
                .get::<EntryFn>(ffi::ENTRY_SYMBOL)
                .map_err(|err| error(format!("not a plugin ({err})")))?;
            let plugin = entry();
            Self::from_ffi(plugin, Some(library)).map_err(|err| error(err.to_string()))
        }
    }

    /// Wraps a plugin object; `library` (if any) is kept loaded as long as needed.
    ///
    /// # Safety
    ///
    /// `plugin` must follow the protocol in [`ffi`], with its code staying loaded
    /// while `library` is (or for the program's lifetime when `library` is `None`).
    pub unsafe fn from_ffi(
        plugin: FfiPlugin,
        library: Option<libloading::Library>,
    ) -> Result<Self, AppError> {
        if plugin.ctx.is_null() {
            return Err(AppError::Plugin("plugin failed to initialize".into()));
        }
        let loaded = Arc::new(Loaded {
            plugin,
            _library: library,
        });
        let plugin = &loaded.plugin;
        // SAFETY: `ctx` is live; returned data is copied before the next call.
        let (name, specs) = unsafe {
            let name = (plugin.name)(plugin.ctx).read()?;
            let specs = (0..(plugin.node_count)(plugin.ctx))
                .map(|index| ffi::read_spec(&(plugin.node_spec)(plugin.ctx, index)))
                .collect::<Result<_, _>>()?;
            (name, specs)
        };
        Ok(Self {
            loaded,
            name,
            specs,
        })
    }
}

impl Plugin for DynamicPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn register(&self, registrar: &mut dyn Registrar) {
        for (index, spec) in self.specs.iter().enumerate() {
            registrar.register_node(Box::new(DynamicProvider {
                loaded: Arc::clone(&self.loaded),
                index,
                spec: spec.clone(),
            }));
        }
    }
}

/// Node type `index` of a [`DynamicPlugin`].
struct DynamicProvider {
    loaded: Arc<Loaded>,
    index: usize,
    spec: NodeSpec,
}

impl NodeProvider for DynamicProvider {
    fn spec(&self) -> NodeSpec {
        self.spec.clone()
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        let plugin = &self.loaded.plugin;
        // SAFETY: `ctx` is live and `index < node_count`.
        let processor = unsafe { (plugin.create_processor)(plugin.ctx, self.index) };
        if processor.ctx.is_null() {
            return Box::new(FailedProcessor);
        }
        Box::new(DynamicProcessor {
            processor,
            _loaded: Arc::clone(&self.loaded),
        })
    }
}

/// A processor living in a plugin library.
struct DynamicProcessor {
    processor: FfiProcessor,
    /// Keeps the plugin (and its library) alive; dropped after `processor`.
    _loaded: Arc<Loaded>,
}

// SAFETY: the plugin-side processor is a `NodeProcessor`, which is `Send`.
unsafe impl Send for DynamicProcessor {}

impl NodeProcessor for DynamicProcessor {
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        let (sample_rate, frames, params, inputs, outputs) = io.raw_parts();
        let args = FfiProcessArgs {
            sample_rate,
            frames,
            params: FfiSlice::new(params),
            inputs: FfiSlice::new(inputs),
            outputs: FfiSlice::new(outputs),
        };
        // SAFETY: `ctx` is live, and the buffers come from a `ProcessIo`, which
        // already guarantees what `ProcessIo::from_raw` needs on the plugin side.
        let status = unsafe { (self.processor.process)(self.processor.ctx, args) };
        if status.ok {
            Ok(())
        } else {
            // SAFETY: the message is valid until the next call on this processor.
            let message = unsafe { status.message.read() }?;
            Err(AppError::Plugin(message))
        }
    }
}

impl Drop for DynamicProcessor {
    fn drop(&mut self) {
        // SAFETY: `ctx` is non-null and dropped only here.
        unsafe { (self.processor.drop)(self.processor.ctx) };
    }
}

/// Stands in for a processor the plugin failed to create (it panicked); every
/// call reports that.
struct FailedProcessor;

impl NodeProcessor for FailedProcessor {
    fn process(&mut self, _io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        Err(AppError::Plugin("plugin failed to create the node".into()))
    }
}
