//! The C ABI between the host and plugins loaded from dynamic libraries.
//!
//! Plugin authors don't use this module directly: [`export_plugin!`](crate::export_plugin)
//! wraps a [`Plugin`] into it, and the host wraps it back.
//!
//! # Protocol
//!
//! 1. The host looks up [`VERSION_SYMBOL`] ([`VersionFn`]) and refuses the library
//!    unless it returns [`API_VERSION`](crate::API_VERSION).
//! 2. It calls [`ENTRY_SYMBOL`] ([`EntryFn`]) once and gets an [`FfiPlugin`]: an
//!    opaque `ctx` plus function pointers taking it. A null `ctx` means the plugin
//!    failed to initialize.
//! 3. It reads the name and every node spec once and copies them
//!    ([`read_spec`]); borrowed data (`FfiStr`, `FfiSlice`) is only valid until
//!    the next call on the same object.
//! 4. Per node instance it calls `create_processor` and drives the returned
//!    [`FfiProcessor`]; it calls each object's `drop` exactly once, processors
//!    before the plugin, and keeps the library loaded until then.
//!
//! Each side frees only what it allocated. Panics never cross the boundary: the
//! plugin side catches them and reports an error (or a null `ctx`) instead. This
//! needs the plugin built with `panic = "unwind"` (the default): with
//! `panic = "abort"` a panic in a plugin aborts the whole host process.
//! Enums travel as `u8` codes and are checked when read.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::{
    AppError, NodeKind, NodeProcessor, NodeProvider, NodeSocket, NodeSpec, ParamKind, ParamSpec,
    ParamValue, Plugin, ProcessIo, RawBuffer, Registrar, SignalEnvironment, SignalType,
    SocketDirection, Translation, slice_from_raw,
};

/// Name of the exported [`VersionFn`].
pub const VERSION_SYMBOL: &[u8] = b"signal_nodes_plugin_api_version\0";
/// Name of the exported [`EntryFn`].
pub const ENTRY_SYMBOL: &[u8] = b"signal_nodes_plugin_entry\0";

/// Returns the API version the library was built against.
pub type VersionFn = unsafe extern "C" fn() -> u32;
/// Creates the plugin; called once per load.
pub type EntryFn = unsafe extern "C" fn() -> FfiPlugin;

/// A borrowed slice.
#[repr(C)]
pub struct FfiSlice<T> {
    pub ptr: *const T,
    pub len: usize,
}

impl<T> Clone for FfiSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for FfiSlice<T> {}

impl<T> FfiSlice<T> {
    pub fn new(items: &[T]) -> Self {
        Self {
            ptr: items.as_ptr(),
            len: items.len(),
        }
    }

    /// # Safety
    ///
    /// `ptr` must point to `len` valid items for `'a` (any pointer if `len == 0`).
    pub unsafe fn as_slice<'a>(self) -> &'a [T] {
        // SAFETY: forwarded to the caller.
        unsafe { slice_from_raw(self.ptr, self.len) }
    }
}

/// A borrowed UTF-8 string (checked when read).
pub type FfiStr = FfiSlice<u8>;

impl FfiStr {
    /// Borrows `text`.
    pub fn of(text: &str) -> Self {
        Self::new(text.as_bytes())
    }

    /// Copies the string out.
    ///
    /// # Safety
    ///
    /// As for [`as_slice`](FfiSlice::as_slice).
    pub unsafe fn read(self) -> Result<String, AppError> {
        // SAFETY: forwarded to the caller.
        let bytes = unsafe { self.as_slice() };
        String::from_utf8(bytes.to_vec())
            .map_err(|_| AppError::Plugin("string is not valid UTF-8".into()))
    }
}

/// A socket of a [`FfiNodeSpec`]; the direction is given by the list it's in.
#[repr(C)]
pub struct FfiSocket {
    pub name: FfiStr,
    pub signal_type: u8,
    pub signal_environment: u8,
}

/// A parameter of a [`FfiNodeSpec`]. Which fields matter depends on `kind`:
/// number — `min`, `max`, `default`; choice — `options`, `default` (index);
/// toggle — `default` (non-zero is on).
#[repr(C)]
pub struct FfiParam {
    pub id: FfiStr,
    pub name: FfiStr,
    pub kind: u8,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub options: FfiSlice<FfiStr>,
}

/// A [`Translation`].
#[repr(C)]
pub struct FfiTranslation {
    pub language: FfiStr,
    pub source: FfiStr,
    pub text: FfiStr,
}

/// A [`NodeSpec`].
#[repr(C)]
pub struct FfiNodeSpec {
    pub type_id: FfiStr,
    pub name: FfiStr,
    pub kind: u8,
    pub inputs: FfiSlice<FfiSocket>,
    pub outputs: FfiSlice<FfiSocket>,
    pub params: FfiSlice<FfiParam>,
    pub translations: FfiSlice<FfiTranslation>,
}

/// A loaded plugin.
#[repr(C)]
pub struct FfiPlugin {
    /// Opaque plugin state; null if the plugin failed to initialize.
    pub ctx: *mut c_void,
    pub name: unsafe extern "C" fn(ctx: *const c_void) -> FfiStr,
    pub node_count: unsafe extern "C" fn(ctx: *const c_void) -> usize,
    /// Spec of node type `index` (`< node_count`).
    pub node_spec: unsafe extern "C" fn(ctx: *const c_void, index: usize) -> FfiNodeSpec,
    /// A new processor for node type `index` (`< node_count`).
    pub create_processor: unsafe extern "C" fn(ctx: *const c_void, index: usize) -> FfiProcessor,
    pub drop: unsafe extern "C" fn(ctx: *mut c_void),
}

/// Arguments of [`FfiProcessor::process`]; the parts of a [`ProcessIo`].
#[repr(C)]
pub struct FfiProcessArgs {
    pub sample_rate: f32,
    pub frames: usize,
    pub params: FfiSlice<ParamValue>,
    pub inputs: FfiSlice<RawBuffer>,
    pub outputs: FfiSlice<RawBuffer>,
}

/// Result of [`FfiProcessor::process`]; on failure `message` holds the error.
#[repr(C)]
pub struct FfiStatus {
    pub ok: bool,
    pub message: FfiStr,
}

/// A node instance's processor.
#[repr(C)]
pub struct FfiProcessor {
    /// Opaque processor state; null if creating it failed.
    pub ctx: *mut c_void,
    /// Must be called with arguments that satisfy [`ProcessIo::from_raw`].
    pub process: unsafe extern "C" fn(ctx: *mut c_void, args: FfiProcessArgs) -> FfiStatus,
    pub drop: unsafe extern "C" fn(ctx: *mut c_void),
}

// --- Enum codes -------------------------------------------------------------

fn node_kind_code(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Generator => 0,
        NodeKind::Processor => 1,
        NodeKind::Output => 2,
    }
}

fn node_kind_from(code: u8) -> Result<NodeKind, AppError> {
    Ok(match code {
        0 => NodeKind::Generator,
        1 => NodeKind::Processor,
        2 => NodeKind::Output,
        _ => return Err(bad_code("node kind", code)),
    })
}

fn signal_type_code(signal_type: SignalType) -> u8 {
    match signal_type {
        SignalType::Analog => 0,
        SignalType::Digital => 1,
        SignalType::DiscreteTime => 2,
        SignalType::Quantized => 3,
    }
}

fn signal_type_from(code: u8) -> Result<SignalType, AppError> {
    Ok(match code {
        0 => SignalType::Analog,
        1 => SignalType::Digital,
        2 => SignalType::DiscreteTime,
        3 => SignalType::Quantized,
        _ => return Err(bad_code("signal type", code)),
    })
}

fn environment_code(environment: SignalEnvironment) -> u8 {
    match environment {
        SignalEnvironment::Electrical => 0,
        SignalEnvironment::Optical => 1,
        SignalEnvironment::Acoustic => 2,
    }
}

fn environment_from(code: u8) -> Result<SignalEnvironment, AppError> {
    Ok(match code {
        0 => SignalEnvironment::Electrical,
        1 => SignalEnvironment::Optical,
        2 => SignalEnvironment::Acoustic,
        _ => return Err(bad_code("signal environment", code)),
    })
}

const PARAM_NUMBER: u8 = 0;
const PARAM_CHOICE: u8 = 1;
const PARAM_TOGGLE: u8 = 2;

fn bad_code(what: &str, code: u8) -> AppError {
    AppError::Plugin(format!("unknown {what} code {code}"))
}

// --- Host side --------------------------------------------------------------

/// Copies a spec received from a plugin.
///
/// # Safety
///
/// Every `FfiStr`/`FfiSlice` in `spec` must be valid (see [`FfiSlice::as_slice`]).
pub unsafe fn read_spec(spec: &FfiNodeSpec) -> Result<NodeSpec, AppError> {
    // SAFETY (whole function): forwarded to the caller.
    unsafe {
        let sockets = |list: FfiSlice<FfiSocket>, direction| -> Result<Vec<NodeSocket>, AppError> {
            list.as_slice()
                .iter()
                .map(|socket| {
                    let name = socket.name.read()?;
                    let signal_type = signal_type_from(socket.signal_type)?;
                    let environment = environment_from(socket.signal_environment)?;
                    Ok(match direction {
                        SocketDirection::Input => NodeSocket::input(name, signal_type, environment),
                        SocketDirection::Output => {
                            NodeSocket::output(name, signal_type, environment)
                        }
                    })
                })
                .collect()
        };
        let params = spec
            .params
            .as_slice()
            .iter()
            .map(|param| {
                let kind = match param.kind {
                    PARAM_NUMBER => ParamKind::Number {
                        min: param.min,
                        max: param.max,
                        default: param.default,
                    },
                    PARAM_CHOICE => ParamKind::Choice {
                        options: param
                            .options
                            .as_slice()
                            .iter()
                            .map(|option| option.read())
                            .collect::<Result<_, _>>()?,
                        default: param.default as usize,
                    },
                    PARAM_TOGGLE => ParamKind::Toggle {
                        default: param.default != 0.0,
                    },
                    code => return Err(bad_code("parameter kind", code)),
                };
                Ok(ParamSpec {
                    id: param.id.read()?,
                    name: param.name.read()?,
                    kind,
                })
            })
            .collect::<Result<_, AppError>>()?;
        let translations = spec
            .translations
            .as_slice()
            .iter()
            .map(|translation| {
                Ok(Translation {
                    language: translation.language.read()?,
                    source: translation.source.read()?,
                    text: translation.text.read()?,
                })
            })
            .collect::<Result<_, AppError>>()?;

        Ok(NodeSpec {
            type_id: spec.type_id.read()?,
            name: spec.name.read()?,
            kind: node_kind_from(spec.kind)?,
            inputs: sockets(spec.inputs, SocketDirection::Input)?,
            outputs: sockets(spec.outputs, SocketDirection::Output)?,
            params,
            translations,
        })
    }
}

// --- Plugin side ------------------------------------------------------------

/// Wraps the plugin built by `make` into the C ABI; used by
/// [`export_plugin!`](crate::export_plugin). Returns a null `ctx` if `make` or
/// the plugin's registration panics.
pub fn export<P: Plugin + 'static>(make: impl FnOnce() -> P) -> FfiPlugin {
    let ctx = catch_unwind(AssertUnwindSafe(|| {
        let plugin = make();
        let mut collector = Collector(Vec::new());
        plugin.register(&mut collector);
        let exported = ExportedPlugin {
            name: plugin.name().to_owned(),
            nodes: collector.0.into_iter().map(ExportedNode::new).collect(),
        };
        Box::into_raw(Box::new(exported)).cast::<c_void>()
    }))
    .unwrap_or(ptr::null_mut());
    FfiPlugin {
        ctx,
        name: plugin_name,
        node_count: plugin_node_count,
        node_spec: plugin_node_spec,
        create_processor: plugin_create_processor,
        drop: plugin_drop,
    }
}

struct Collector(Vec<Box<dyn NodeProvider>>);

impl Registrar for Collector {
    fn register_node(&mut self, provider: Box<dyn NodeProvider>) {
        self.0.push(provider);
    }
}

/// Plugin state behind [`FfiPlugin::ctx`].
struct ExportedPlugin {
    name: String,
    nodes: Vec<ExportedNode>,
}

/// One node type with its spec kept alive in C layout.
///
/// The `Ffi*` vectors point into `spec`'s strings and into each other. That is
/// fine because none of them is modified after construction, so their heap
/// buffers never move (moving the struct only moves the vector headers).
struct ExportedNode {
    spec: NodeSpec,
    inputs: Vec<FfiSocket>,
    outputs: Vec<FfiSocket>,
    /// Option labels per parameter (empty for non-choice parameters).
    _options: Vec<Vec<FfiStr>>,
    params: Vec<FfiParam>,
    translations: Vec<FfiTranslation>,
    provider: Box<dyn NodeProvider>,
}

impl ExportedNode {
    fn new(provider: Box<dyn NodeProvider>) -> Self {
        let spec = provider.spec();
        let sockets = |sockets: &[NodeSocket]| -> Vec<FfiSocket> {
            sockets
                .iter()
                .map(|socket| FfiSocket {
                    name: FfiStr::of(&socket.name),
                    signal_type: signal_type_code(socket.signal_type),
                    signal_environment: environment_code(socket.signal_environment),
                })
                .collect()
        };
        let options: Vec<Vec<FfiStr>> = spec
            .params
            .iter()
            .map(|param| match &param.kind {
                ParamKind::Choice { options, .. } => {
                    options.iter().map(|option| FfiStr::of(option)).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        let params = spec
            .params
            .iter()
            .zip(&options)
            .map(|(param, options)| {
                let (kind, min, max, default) = match &param.kind {
                    ParamKind::Number { min, max, default } => (PARAM_NUMBER, *min, *max, *default),
                    ParamKind::Choice { default, .. } => (PARAM_CHOICE, 0.0, 0.0, *default as f32),
                    ParamKind::Toggle { default } => {
                        (PARAM_TOGGLE, 0.0, 0.0, f32::from(u8::from(*default)))
                    }
                };
                FfiParam {
                    id: FfiStr::of(&param.id),
                    name: FfiStr::of(&param.name),
                    kind,
                    min,
                    max,
                    default,
                    options: FfiSlice::new(options),
                }
            })
            .collect();
        let translations = spec
            .translations
            .iter()
            .map(|translation| FfiTranslation {
                language: FfiStr::of(&translation.language),
                source: FfiStr::of(&translation.source),
                text: FfiStr::of(&translation.text),
            })
            .collect();

        Self {
            inputs: sockets(&spec.inputs),
            outputs: sockets(&spec.outputs),
            _options: options,
            params,
            translations,
            spec,
            provider,
        }
    }

    fn ffi_spec(&self) -> FfiNodeSpec {
        FfiNodeSpec {
            type_id: FfiStr::of(&self.spec.type_id),
            name: FfiStr::of(&self.spec.name),
            kind: node_kind_code(self.spec.kind),
            inputs: FfiSlice::new(&self.inputs),
            outputs: FfiSlice::new(&self.outputs),
            params: FfiSlice::new(&self.params),
            translations: FfiSlice::new(&self.translations),
        }
    }
}

/// # Safety
///
/// `ctx` must be a live, non-null [`FfiPlugin::ctx`].
unsafe fn plugin<'a>(ctx: *const c_void) -> &'a ExportedPlugin {
    // SAFETY: forwarded to the caller.
    unsafe { &*ctx.cast::<ExportedPlugin>() }
}

unsafe extern "C" fn plugin_name(ctx: *const c_void) -> FfiStr {
    // SAFETY: the host passes the ctx it got from `export`.
    FfiStr::of(unsafe { &plugin(ctx).name })
}

unsafe extern "C" fn plugin_node_count(ctx: *const c_void) -> usize {
    // SAFETY: as in `plugin_name`.
    unsafe { plugin(ctx).nodes.len() }
}

unsafe extern "C" fn plugin_node_spec(ctx: *const c_void, index: usize) -> FfiNodeSpec {
    // SAFETY: as in `plugin_name`; the host keeps `index < node_count`.
    unsafe { plugin(ctx).nodes[index].ffi_spec() }
}

unsafe extern "C" fn plugin_create_processor(ctx: *const c_void, index: usize) -> FfiProcessor {
    let processor = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `plugin_node_spec`.
        let node = unsafe { &plugin(ctx).nodes[index] };
        node.provider.create_processor()
    }));
    let ctx = match processor {
        Ok(processor) => Box::into_raw(Box::new(ExportedProcessor {
            processor,
            error: String::new(),
        }))
        .cast::<c_void>(),
        Err(_) => ptr::null_mut(),
    };
    FfiProcessor {
        ctx,
        process: processor_process,
        drop: processor_drop,
    }
}

unsafe extern "C" fn plugin_drop(ctx: *mut c_void) {
    // SAFETY: the host calls this once with the ctx it got from `export`.
    let plugin = unsafe { Box::from_raw(ctx.cast::<ExportedPlugin>()) };
    let _ = catch_unwind(AssertUnwindSafe(move || drop(plugin)));
}

/// Processor state behind [`FfiProcessor::ctx`].
struct ExportedProcessor {
    processor: Box<dyn NodeProcessor>,
    /// Message of the last failed `process`, borrowed by the returned status.
    error: String,
}

unsafe extern "C" fn processor_process(ctx: *mut c_void, args: FfiProcessArgs) -> FfiStatus {
    // SAFETY: the host passes the ctx it got from `create_processor`, and
    // arguments that satisfy `ProcessIo::from_raw` (see `FfiProcessor::process`).
    let state = unsafe { &mut *ctx.cast::<ExportedProcessor>() };
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let mut io = ProcessIo::from_raw(
            args.sample_rate,
            args.frames,
            args.params.as_slice(),
            args.inputs.as_slice(),
            args.outputs.as_slice(),
        );
        state.processor.process(&mut io)
    }));
    state.error = match result {
        Ok(Ok(())) => {
            return FfiStatus {
                ok: true,
                message: FfiStr::of(""),
            };
        }
        Ok(Err(err)) => err.to_string(),
        Err(_) => "node panicked while processing".to_owned(),
    };
    FfiStatus {
        ok: false,
        message: FfiStr::of(&state.error),
    }
}

unsafe extern "C" fn processor_drop(ctx: *mut c_void) {
    // SAFETY: the host calls this once with the ctx it got from `create_processor`.
    let processor = unsafe { Box::from_raw(ctx.cast::<ExportedProcessor>()) };
    let _ = catch_unwind(AssertUnwindSafe(move || drop(processor)));
}
