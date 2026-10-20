//! Bottom debug panel.
//!
//! Shows, top to bottom:
//! - a node-limit warning when the last add failed (see `node_limit_reached`);
//! - a warning when the last connection was refused as a cycle (`connection_refused`);
//! - the selected source socket, or "Ready" when nothing is selected and there
//!   are no warnings;
//! - every connection as `from_node Output n -> to_node Input m`, sorted by source
//!   and scrollable once it exceeds [`CONNECTIONS_MAX_HEIGHT`].
//!
//! Node IDs are shown as-is (0-based), socket numbers 1-based. Read-only: the
//! panel never changes the app state.

use eframe::egui;

use super::{TEXT_DIM, caption, panel_frame};
use crate::editor::{MAX_NODE_COUNT, Text, VSignalNodesApp};

/// Height after which the connection list scrolls instead of growing the panel.
const CONNECTIONS_MAX_HEIGHT: f32 = 120.0;

/// Shows the debug panel along the bottom of the window.
pub(super) fn show(app: &VSignalNodesApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("debug")
        .resizable(false)
        .frame(panel_frame(super::PANEL_BG))
        .show(ui, |ui| {
            caption(ui, app.language.text(Text::Debug));
            if app.node_limit_reached {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "{}: {}",
                        app.language.text(Text::NodeLimitReached),
                        MAX_NODE_COUNT,
                    ),
                );
            }
            if app.connection_refused {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    app.language.text(Text::ConnectionCreatesCycle),
                );
            }
            if let Some(source) = app.pending_source {
                ui.label(format!(
                    "{} {} {} {}",
                    app.language.text(Text::SelectedSourceSocket),
                    source.node_id,
                    source.side.label(app.language),
                    source.index + 1,
                ));
            } else if !app.node_limit_reached && !app.connection_refused {
                ui.label(app.language.text(Text::Ready));
            }
            let mut connections: Vec<_> = app.connections.iter().collect();
            connections.sort_by_key(|(to, from)| (from.node_id, from.index, to.node_id, to.index));
            egui::ScrollArea::vertical()
                .max_height(CONNECTIONS_MAX_HEIGHT)
                .show(ui, |ui| {
                    for (to, from) in connections {
                        let line = format!(
                            "{} {} {} -> {} {} {}",
                            from.node_id,
                            from.side.label(app.language),
                            from.index + 1,
                            to.node_id,
                            to.side.label(app.language),
                            to.index + 1,
                        );
                        ui.label(
                            egui::RichText::new(line)
                                .monospace()
                                .size(12.0)
                                .color(TEXT_DIM),
                        );
                    }
                });
        });
}
