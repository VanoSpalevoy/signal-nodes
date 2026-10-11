//! Left node palette.
//!
//! One button per [`NodeKind`]. Dragging a button starts a palette drag by setting
//! `app.dragged_palette_kind`; the canvas draws the drop preview and creates the
//! node when the button is released over it (see `canvas::finish_palette_drop`).
//! Releasing anywhere else cancels the drag. Clicking a button does nothing.

use eframe::egui;

use super::{node_color, panel_frame};
use crate::editor::{NodeKind, SignalForgeApp};

/// Shows the palette as a fixed-width (120 pt) left panel.
pub(super) fn show(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    egui::Panel::left("palette")
        .resizable(false)
        .frame(panel_frame(super::PANEL_BG))
        .show(ui, |ui| {
            ui.set_min_width(120.0);
            ui.set_max_width(120.0);
            ui.vertical(|ui| {
                ui.heading(app.language.text(crate::editor::Text::Nodes));
                ui.separator();
                for kind in [
                    NodeKind::Source,
                    NodeKind::Gain,
                    NodeKind::Mixer,
                    NodeKind::Sink,
                ] {
                    let response = ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::Button::new(
                            egui::RichText::new(kind.label(app.language)).color(super::TEXT),
                        )
                        .fill(node_color(kind))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::WHITE))
                        .sense(egui::Sense::click_and_drag()),
                    );

                    if response.drag_started() {
                        app.dragged_palette_kind = Some(kind);
                    }

                    if app.dragged_palette_kind == Some(kind) {
                        ui.ctx().output_mut(|output| {
                            output.cursor_icon = egui::CursorIcon::Grab;
                        });
                    }
                }
            });
        });
}
