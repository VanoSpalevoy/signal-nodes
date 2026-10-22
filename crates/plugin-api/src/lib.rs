//! The API a plugin implements to provide node types.
//!
//! A [`Plugin`] registers one [`NodeProvider`] per node type through a
//! [`Registrar`]. A provider describes its node type with a [`NodeSpec`] and
//! creates a fresh [`NodeProcessor`] (the DSP state) for every node instance.
//!
//! The same traits serve both ways of loading a plugin:
//!
//! - **in-process** — the host links the plugin crate and passes the [`Plugin`]
//!   value to `PluginHost::load`;
//! - **dynamic library** — the plugin crate is built as a `cdylib` and calls
//!   [`export_plugin!`]; the host loads it with `PluginHost::load_library`. Rust
//!   trait objects aren't ABI-stable, so everything crosses the library boundary
//!   through the C ABI in [`ffi`], and the host wraps it back into these traits.
//!
//! ```ignore
//! use v_signal_nodes_plugin_api::*;
//!
//! struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn name(&self) -> &str { "my" }
//!     fn register(&self, registrar: &mut dyn Registrar) {
//!         registrar.register_node(Box::new(MyGain));
//!     }
//! }
//!
//! export_plugin!(MyPlugin);
//! ```

// This is the contract plugin authors build against: everything public is documented.
#![warn(missing_docs)]

use std::marker::PhantomData;
use std::slice;

pub mod ffi;

pub use v_signal_nodes_core::{
    AppError, NodeKind, NodeSocket, NodeSpec, ParamKind, ParamSpec, ParamValue, SampleBlock,
    SignalEnvironment, SignalType, SocketDirection, Translation,
};

/// Version of this API. The host refuses plugins built against another version.
///
/// Bump it on any change to the traits or to the [`ffi`] types.
pub const API_VERSION: u32 = 1;

/// Per-instance processing state of a node.
pub trait NodeProcessor: Send {
    /// Processes one block. See [`ProcessIo`] for what's available.
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError>;
}

/// One node type offered by a plugin.
pub trait NodeProvider: Send + Sync {
    /// The node type's description. Its `type_id` should be namespaced by the
    /// plugin, e.g. `"std.gain"`.
    fn spec(&self) -> NodeSpec;

    /// Creates the processing state for a new node instance.
    fn create_processor(&self) -> Box<dyn NodeProcessor>;
}

/// Collects the node types a plugin offers.
pub trait Registrar {
    /// Adds one node type.
    fn register_node(&mut self, provider: Box<dyn NodeProvider>);
}

/// A plugin: a named bundle of node types.
pub trait Plugin {
    /// Unique plugin name, e.g. `"std"`.
    fn name(&self) -> &str;

    /// The [`API_VERSION`] the plugin was built against.
    fn api_version(&self) -> u32 {
        API_VERSION
    }

    /// Registers all node types of the plugin.
    fn register(&self, registrar: &mut dyn Registrar);
}

/// A sample buffer passed to [`NodeProcessor::process`], as a raw pointer and
/// length so it can cross the C ABI unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RawBuffer {
    /// First sample.
    pub ptr: *mut f32,
    /// Number of samples.
    pub len: usize,
}

impl RawBuffer {
    /// Points at `samples` (read-only use: an input).
    pub fn input(samples: &[f32]) -> Self {
        Self {
            ptr: samples.as_ptr().cast_mut(),
            len: samples.len(),
        }
    }

    /// Points at `samples` (writable: an output).
    pub fn output(samples: &mut [f32]) -> Self {
        Self {
            ptr: samples.as_mut_ptr(),
            len: samples.len(),
        }
    }
}

/// Everything a node gets for one [`process`](NodeProcessor::process) call:
/// parameter values, input samples and output buffers, all `frames` long.
///
/// Inputs and outputs are in socket order of the node's [`NodeSpec`]; parameters in
/// the order of [`NodeSpec::params`]. Outputs start zeroed.
pub struct ProcessIo<'a> {
    sample_rate: f32,
    frames: usize,
    params: &'a [ParamValue],
    inputs: &'a [RawBuffer],
    outputs: &'a [RawBuffer],
    _samples: PhantomData<&'a mut [f32]>,
}

impl<'a> ProcessIo<'a> {
    /// # Safety
    ///
    /// For `'a`, every buffer in `inputs` and `outputs` must point to `frames`
    /// initialized samples (`len == frames`), the outputs must be writable, and no
    /// output may overlap another output or any input.
    pub unsafe fn from_raw(
        sample_rate: f32,
        frames: usize,
        params: &'a [ParamValue],
        inputs: &'a [RawBuffer],
        outputs: &'a [RawBuffer],
    ) -> Self {
        Self {
            sample_rate,
            frames,
            params,
            inputs,
            outputs,
            _samples: PhantomData,
        }
    }

    /// Samples per second.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Number of samples in every input and output.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// All parameter values.
    pub fn params(&self) -> &'a [ParamValue] {
        self.params
    }

    /// Parameter `index` as a number; `0.0` if it is missing or not a number.
    pub fn number(&self, index: usize) -> f32 {
        self.params
            .get(index)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0)
    }

    /// Parameter `index` as a choice; `0` if it is missing or not a choice.
    pub fn choice(&self, index: usize) -> usize {
        self.params
            .get(index)
            .and_then(|value| value.as_choice())
            .unwrap_or(0)
    }

    /// Parameter `index` as a toggle; `false` if it is missing or not a toggle.
    pub fn toggle(&self, index: usize) -> bool {
        self.params
            .get(index)
            .and_then(|value| value.as_toggle())
            .unwrap_or(false)
    }

    /// Number of inputs.
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// Number of outputs.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Samples of input `index`. Panics if there's no such input.
    ///
    /// Not tied to `&self`, so an input can be read while an output is written.
    pub fn input(&self, index: usize) -> &'a [f32] {
        let buffer = self.inputs[index];
        // SAFETY: `from_raw` guarantees a valid, initialized buffer for `'a` that
        // no output overlaps, so a shared slice can't alias a `&mut`.
        unsafe { slice_from_raw(buffer.ptr, buffer.len) }
    }

    /// Buffer of output `index`. Panics if there's no such output.
    pub fn output(&mut self, index: usize) -> &mut [f32] {
        let buffer = self.outputs[index];
        // SAFETY: `from_raw` guarantees a valid, writable buffer for `'a` that
        // overlaps nothing else; borrowing `self` mutably hands out one at a time.
        unsafe {
            if buffer.len == 0 {
                &mut []
            } else {
                slice::from_raw_parts_mut(buffer.ptr, buffer.len)
            }
        }
    }

    /// The raw parts, for hosts that forward the call (e.g. across the C ABI).
    pub fn raw_parts(
        &mut self,
    ) -> (
        f32,
        usize,
        &'a [ParamValue],
        &'a [RawBuffer],
        &'a [RawBuffer],
    ) {
        (
            self.sample_rate,
            self.frames,
            self.params,
            self.inputs,
            self.outputs,
        )
    }
}

/// `slice::from_raw_parts` that accepts a null/dangling pointer for an empty slice.
///
/// # Safety
///
/// Same as [`slice::from_raw_parts`] when `len > 0`.
unsafe fn slice_from_raw<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if len == 0 {
        &[]
    } else {
        // SAFETY: forwarded to the caller.
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

/// Exports a plugin from a `cdylib` crate, so the host can load it with
/// `PluginHost::load_library`.
///
/// Takes an expression that builds the plugin (a `'static` [`Plugin`]). Call it
/// once per library, at the crate root; it defines the entry symbols named in
/// [`ffi`].
#[macro_export]
macro_rules! export_plugin {
    ($plugin:expr) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn v_signal_nodes_plugin_api_version() -> u32 {
            $crate::API_VERSION
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn v_signal_nodes_plugin_entry() -> $crate::ffi::FfiPlugin {
            $crate::ffi::export(|| $plugin)
        }
    };
}
