//! Signal Forge editor binary: opens the native window and runs [`editor::SignalForgeApp`].
//!
//! Release builds on Windows use the GUI subsystem, so no console window appears.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod editor;

fn main() -> eframe::Result {
    eframe::run_native(
        "Signal Forge",
        eframe::NativeOptions::default(),
        Box::new(|creation_context| {
            creation_context
                .egui_ctx
                .set_visuals(eframe::egui::Visuals::dark());
            Ok(Box::<editor::SignalForgeApp>::default())
        }),
    )
}
