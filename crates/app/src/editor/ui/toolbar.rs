//! Top toolbar.
//!
//! - **Format** — snaps every node's top-left corner to a [`GRID_STEP`] grid
//!   anchored at the canvas origin.
//! - **Scale** — canvas zoom (0.5–2.0) stored in `app.scale`; not applied to rendering yet.
//! - **Language** — switches the UI language; takes effect on the next frame.
//!
//! The toolbar wraps onto more lines when the window is narrow, which makes the
//! panel taller and the canvas shorter.

use eframe::egui;
use egui::Widget;

use super::{Language, panel_frame};
use crate::editor::{SignalForgeApp, Text};

/// Grid size used by the Format button, in canvas points.
const GRID_STEP: f32 = 50.0;

/// Shows the toolbar as a top panel and applies its actions to `app`.
pub(super) fn show(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar")
        .resizable(false)
        .frame(panel_frame(super::PANEL_BG))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(34.0);
                ui.add_space(12.0);
                ui.label(app.language.text(Text::Tools));
                ui.add_space(12.0);
                if ui.button(app.language.text(Text::Format)).clicked() {
                    // Node positions are canvas coordinates, so the grid starts at the canvas origin.
                    for node in &mut app.nodes {
                        node.pos = (node.pos / GRID_STEP).round() * GRID_STEP;
                    }
                }
                ui.separator();
                egui::Slider::new(&mut app.scale, 0.5..=2.0)
                    .text(app.language.text(Text::Scale))
                    .show_value(true)
                    .ui(ui);
                ui.separator();
                ui.label(app.language.text(Text::Settings));
                ui.add_space(12.0);
                ui.label(app.language.text(Text::Language));
                egui::ComboBox::from_id_salt("language")
                    .selected_text(app.language.native_name())
                    .show_ui(ui, |ui| {
                        for language in Language::ALL {
                            ui.selectable_value(
                                &mut app.language,
                                language,
                                language.native_name(),
                            );
                        }
                    });
            });
        });
}
