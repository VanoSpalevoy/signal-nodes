//! Editor UI: panels, the node canvas and the editor-side node model.
//!
//! The UI is immediate mode: [`render`] runs every frame, draws everything from
//! [`SignalForgeApp`] and applies that frame's input to it directly. No widget state
//! is kept outside the app (apart from small per-widget scratch in egui memory).
//!
//! # Layout
//!
//! ```text
//! ┌──────────────────────── toolbar ─────────────────────────┐
//! ├─────────┬────────────────────────────────────────────────┤
//! │ palette │ canvas (scrolls both ways)                    ▐│
//! │         │                                               ▐│
//! │         │▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄│
//! ├─────────┴────────────────────────────────────────────────┤
//! │ debug                                                    │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! | Module      | Panel           | Role                                               |
//! |-------------|-----------------|----------------------------------------------------|
//! | [`toolbar`] | top             | grid snapping, scale, language                     |
//! | [`debug`]   | bottom          | status line and the list of connections            |
//! | [`palette`] | left            | node kinds to drag onto the canvas                 |
//! | [`canvas`]  | central         | nodes and connections: drag, connect, delete       |
//! | [`node`]    | —               | node model and geometry ([`NodeWidget`], sockets)  |
//!
//! # Conventions
//!
//! - Panel order in [`render`] matters: egui panels added earlier claim their space
//!   first, which is why the debug panel spans the full width under the palette.
//! - Colors are the constants below; panels share [`panel_frame`].
//! - All visible text goes through [`Language::text`].

mod canvas;
mod debug;
mod node;
mod palette;
mod toolbar;

use eframe::egui;

use super::SignalForgeApp;
pub(super) use super::localization::Language;
pub use node::{NodeConfig, NodeKind, NodeWidget, SocketKind, SocketRef};

/// Darkest background: text edits and other "extreme" backgrounds.
const WINDOW_BG: egui::Color32 = egui::Color32::from_rgb(10, 12, 16);
/// Background of the side panels (toolbar, palette, debug).
const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);
/// Background of the node canvas.
const CANVAS_BG: egui::Color32 = egui::Color32::from_rgb(20, 24, 32);
/// Resting fill of buttons, sliders and other controls.
const CONTROL_BG: egui::Color32 = egui::Color32::from_rgb(31, 36, 45);
/// Fill of a hovered control.
const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(45, 53, 65);
/// Default text color; forced on all widgets via `override_text_color`.
const TEXT: egui::Color32 = egui::Color32::from_rgb(232, 235, 240);
/// Panel border stroke.
const BORDER: egui::Color32 = egui::Color32::from_rgb(48, 54, 64);

/// Draws the whole editor for one frame: theme, then panels in layout order.
pub(super) fn render(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    ui.ctx().set_visuals(visuals());
    toolbar::show(app, ui);
    debug::show(app, ui);
    palette::show(app, ui);
    canvas::show(app, ui);
}

/// Frame shared by all panels: `fill` background, [`BORDER`] outline, 8 pt padding.
pub(super) fn panel_frame(fill: egui::Color32) -> egui::Frame {
    egui::Frame::default()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::same(8))
}

/// The editor's dark theme: egui's dark visuals recolored with the constants above.
fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = PANEL_BG;
    visuals.panel_fill = PANEL_BG;
    visuals.extreme_bg_color = WINDOW_BG;
    visuals.faint_bg_color = CANVAS_BG;
    visuals.code_bg_color = WINDOW_BG;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = PANEL_BG;
    visuals.widgets.inactive.bg_fill = CONTROL_BG;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = CONTROL_HOVER;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(59, 73, 91);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals
}

/// Fill color of a node kind; the palette button, the drop preview and the node
/// itself all use it, so a kind is recognizable everywhere.
fn node_color(kind: NodeKind) -> egui::Color32 {
    match kind {
        NodeKind::Source => egui::Color32::from_rgb(75, 142, 88),
        NodeKind::Gain => egui::Color32::from_rgb(110, 92, 188),
        NodeKind::Mixer => egui::Color32::from_rgb(56, 128, 145),
        NodeKind::Sink => egui::Color32::from_rgb(165, 90, 72),
    }
}
