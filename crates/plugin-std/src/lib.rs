//! The standard plugin: built-in node types (`std.*`).
//!
//! Placeholder for now: [`StdPlugin`] registers no node types yet.

use signal_nodes_plugin_api::{Plugin, Registrar};

/// The standard plugin, named `"std"`.
pub struct StdPlugin;

impl Plugin for StdPlugin {
    fn name(&self) -> &str {
        "std"
    }

    fn register(&self, _registrar: &mut dyn Registrar) {}
}
