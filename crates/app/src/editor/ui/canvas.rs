//! Central node canvas: draws nodes and connections and handles all node interaction.
//!
//! # Coordinates
//!
//! Nodes store **canvas coordinates** (`NodeWidget::pos`): `(0, 0)` is the top-left
//! of the scrollable content. egui works in **screen coordinates**. In each frame
//! `origin` is where the canvas `(0, 0)` currently is on screen:
//!
//! ```text
//! screen = canvas + origin        canvas = screen - origin
//! ```
//!
//! Interaction rects and painting use screen coordinates (via [`node_rect`],
//! [`socket_pos`], [`socket_rect`], [`delete_button_rect`]); anything stored in
//! the app uses canvas coordinates.
//!
//! # Scrolling and size
//!
//! The canvas is a [`egui::ScrollArea::both`] with solid, always-visible scroll
//! bars at the bottom and right. Its content is never smaller than the viewport
//! and grows to fit the furthest node plus [`CANVAS_MARGIN`] (see [`content_size`]),
//! so moving a node right or down extends the canvas. Nodes can't go above or left
//! of `(0, 0)`.
//!
//! # Interaction
//!
//! | Input                                   | Effect                                          |
//! |-----------------------------------------|-------------------------------------------------|
//! | drag node body                          | move the node (auto-scrolls near the edges)     |
//! | primary click output socket             | select it as the pending source                 |
//! | primary click input socket              | connect the pending source to it               |
//! | secondary click output socket           | remove every connection from that output        |
//! | secondary click input socket            | remove the connection into that input           |
//! | primary click `x`                       | delete the node and its connections             |
//! | drop a palette button on the canvas     | add a node centered on the pointer              |
//!
//! Connecting is refused (and the selection cleared) when the input is already
//! connected or belongs to the same node as the source.
//!
//! # Frame order
//!
//! 1. Allocate the content rect, which fixes `origin` for this frame.
//! 2. Finish a palette drop, then handle node input ([`handle_node_interactions`]).
//!    Widgets created later win overlapping clicks, so sockets and the delete
//!    button sit above their node body, and later nodes above earlier ones.
//! 3. Paint connections, then nodes, then the palette drop preview on top.

use eframe::egui;
use std::collections::HashMap;

use super::{Language, NodeConfig, NodeKind, node_color, panel_frame};
use crate::editor::{NodeWidget, SignalForgeApp, SocketKind, SocketRef, Text};

/// Side of the square delete button in a node's top-left corner.
const DELETE_BUTTON_SIZE: f32 = 22.0;
/// Free space kept to the right of and below the furthest node, so the canvas can grow.
const CANVAS_MARGIN: f32 = 200.0;
/// Distance from the viewport edge at which dragging a node starts scrolling.
const EDGE_SCROLL_ZONE: f32 = 32.0;
/// Maximum edge-scroll speed, in points per second.
const EDGE_SCROLL_SPEED: f32 = 600.0;

/// Shows the canvas in the central panel, inside a scroll area.
pub(super) fn show(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        .frame(panel_frame(super::CANVAS_BG))
        .show(ui, |ui| {
            // Solid bars get their own strip at the bottom and right instead of
            // floating over the nodes and showing up only on hover.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            egui::ScrollArea::both()
                .id_salt("canvas")
                .auto_shrink(false)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| show_content(app, ui));
        });
}

/// One frame of the scrollable canvas content; see the module docs for the order of steps.
fn show_content(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    let viewport = ui.clip_rect();
    let (canvas_rect, _) = ui.allocate_exact_size(
        content_size(&app.nodes, viewport.size()),
        egui::Sense::hover(),
    );
    let origin = canvas_rect.min.to_vec2();
    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| viewport.contains(*pos));

    finish_palette_drop(app, ui, origin, pointer);
    handle_node_interactions(app, ui, origin, viewport);

    let painter = ui.painter();
    paint_connections(&app.connections, &app.nodes, painter, origin);
    paint_nodes(&app.nodes, painter, app.language, origin);
    paint_palette_preview(app, painter, origin, pointer);
}

/// Content fills at least the viewport and grows to fit every node plus a margin.
fn content_size(nodes: &[NodeWidget], viewport: egui::Vec2) -> egui::Vec2 {
    nodes.iter().fold(viewport, |size, node| {
        size.max(node.rect().max.to_vec2() + egui::Vec2::splat(CANVAS_MARGIN))
    })
}

/// Canvas rect of a node of `kind` dropped at screen `pointer`: centered on it, kept off negative coordinates.
fn drop_rect(kind: NodeKind, pointer: egui::Pos2, origin: egui::Vec2) -> egui::Rect {
    let size = NodeConfig::for_kind(kind).size();
    let pos = (pointer - origin - size * 0.5).max(egui::Pos2::ZERO);
    egui::Rect::from_min_size(pos, size)
}

/// Ends a palette drag on pointer release: adds the node if the pointer is over
/// the canvas viewport (`pointer` is `None` otherwise), and cancels the drag either way.
fn finish_palette_drop(
    app: &mut SignalForgeApp,
    ui: &egui::Ui,
    origin: egui::Vec2,
    pointer: Option<egui::Pos2>,
) {
    if let Some(kind) = app.dragged_palette_kind {
        if ui.ctx().input(|input| input.pointer.any_released()) {
            if let Some(pos) = pointer {
                app.add_node_at(kind, drop_rect(kind, pos, origin).min);
            }
            app.dragged_palette_kind = None;
        }
    }
}

/// While a node is dragged with the pointer near the viewport edge, scrolls the view that way.
///
/// Speed grows linearly from 0 at [`EDGE_SCROLL_ZONE`] inside the edge to
/// [`EDGE_SCROLL_SPEED`] at the edge and beyond, and is scaled by frame time so it
/// doesn't depend on the frame rate. Requests repaints while scrolling, so the view
/// keeps moving with the pointer held still. The dragged node follows on its own,
/// since [`drag_node`] re-anchors it to the pointer every frame.
fn edge_scroll(ui: &egui::Ui, viewport: egui::Rect, pointer: egui::Pos2) {
    let axis_speed = |pos: f32, min: f32, max: f32| {
        if pos > max - EDGE_SCROLL_ZONE {
            ((pos - (max - EDGE_SCROLL_ZONE)) / EDGE_SCROLL_ZONE).min(1.0)
        } else if pos < min + EDGE_SCROLL_ZONE {
            -((min + EDGE_SCROLL_ZONE - pos) / EDGE_SCROLL_ZONE).min(1.0)
        } else {
            0.0
        }
    };
    let dt = ui.input(|input| input.stable_dt).min(0.1);
    let delta = egui::vec2(
        axis_speed(pointer.x, viewport.left(), viewport.right()),
        axis_speed(pointer.y, viewport.top(), viewport.bottom()),
    ) * EDGE_SCROLL_SPEED
        * dt;

    if delta != egui::Vec2::ZERO {
        // A negative scroll delta moves the content up/left, i.e. the view down/right.
        // The scroll area clamps the offset to the content bounds itself.
        ui.scroll_with_delta_animation(-delta, egui::style::ScrollAnimation::none());
        ui.ctx().request_repaint();
    }
}

/// Moves a dragged node so the point that was grabbed stays exactly under the pointer.
///
/// The position is absolute rather than accumulated from drag deltas, so the node
/// can't drift away from the cursor when the view scrolls mid-drag (edge scroll,
/// mouse wheel or trackpad).
fn drag_node(
    ui: &egui::Ui,
    node: &mut NodeWidget,
    response: &egui::Response,
    origin: egui::Vec2,
    viewport: egui::Rect,
) {
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    if response.drag_started() {
        // Measure the grab offset from where the button went down, before the drag threshold moved the pointer.
        let press = ui
            .input(|input| input.pointer.press_origin())
            .unwrap_or(pointer);
        let grab = press - origin - node.pos;
        ui.data_mut(|data| data.insert_temp(response.id, grab));
    }
    if response.dragged() {
        let grab: egui::Vec2 = ui
            .data(|data| data.get_temp(response.id))
            .unwrap_or_default();
        node.pos = pointer - origin - grab;
        node.clamp_to_canvas();
        edge_scroll(ui, viewport, pointer);
    }
}

/// Registers the interactive areas of every node and applies this frame's clicks
/// and drags: moving, selecting a source, connecting, disconnecting and deleting.
///
/// At most one node is deleted per frame; it is removed after the loop together
/// with its connections, its ID goes back to the pool and a pending source on it
/// is cleared.
fn handle_node_interactions(
    app: &mut SignalForgeApp,
    ui: &mut egui::Ui,
    origin: egui::Vec2,
    viewport: egui::Rect,
) {
    let mut deleted_node_id = None;

    for node_index in 0..app.nodes.len() {
        let node = &mut app.nodes[node_index];
        // Stable per-node id: the grab offset is stored under it for the whole drag.
        let drag_response = ui.interact(
            node_rect(node, origin),
            ui.id().with(("canvas_node", node.id)),
            egui::Sense::drag(),
        );
        drag_node(ui, node, &drag_response, origin, viewport);

        let input_responses: Vec<_> = (0..node.config.inputs.len())
            .map(|index| {
                ui.allocate_rect(
                    socket_rect(node, SocketKind::Input, index, origin),
                    egui::Sense::click(),
                )
            })
            .collect();
        let output_responses: Vec<_> = (0..node.config.outputs.len())
            .map(|index| {
                ui.allocate_rect(
                    socket_rect(node, SocketKind::Output, index, origin),
                    egui::Sense::click(),
                )
            })
            .collect();
        let delete_response = ui
            .allocate_rect(delete_button_rect(node, origin), egui::Sense::click())
            .on_hover_text(app.language.text(Text::DeleteNode));

        if delete_response.clicked_by(egui::PointerButton::Primary) {
            deleted_node_id = Some(node.id);
            continue;
        }

        if let Some(index) = output_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Primary))
        {
            app.pending_source = Some(SocketRef {
                node_id: node.id,
                side: SocketKind::Output,
                index,
            });
        }

        if let Some(index) = input_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Primary))
        {
            if let Some(from) = app.pending_source {
                if from.node_id != node.id && from.side == SocketKind::Output {
                    let input = SocketRef {
                        node_id: node.id,
                        side: SocketKind::Input,
                        index,
                    };
                    if app.connections.contains_key(&input) {
                        app.pending_source = None;
                        continue;
                    }
                    app.connections.insert(input, from);
                }
                app.pending_source = None;
            }
        }

        if let Some(index) = output_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Secondary))
        {
            app.connections.retain(|_, from| {
                !(from.node_id == node.id && from.side == SocketKind::Output && from.index == index)
            });
        }

        if let Some(index) = input_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Secondary))
        {
            app.connections.remove(&SocketRef {
                node_id: node.id,
                side: SocketKind::Input,
                index,
            });
        }
    }

    if let Some(node_id) = deleted_node_id {
        app.nodes.retain(|node| node.id != node_id);
        app.connections
            .retain(|input, output| input.node_id != node_id && output.node_id != node_id);
        app.free_node_ids.push(node_id);
        app.node_limit_reached = false;
        if app
            .pending_source
            .is_some_and(|source| source.node_id == node_id)
        {
            app.pending_source = None;
        }
    }
}

/// Draws a translucent node of the dragged kind exactly where it would be dropped.
fn paint_palette_preview(
    app: &SignalForgeApp,
    painter: &egui::Painter,
    origin: egui::Vec2,
    pointer: Option<egui::Pos2>,
) {
    if let (Some(kind), Some(pos)) = (app.dragged_palette_kind, pointer) {
        let preview_rect = drop_rect(kind, pos, origin).translate(origin);
        painter.rect_filled(preview_rect, 10.0, node_color(kind).gamma_multiply(0.65));
        painter.rect_stroke(
            preview_rect,
            10.0,
            egui::Stroke::new(1.2, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            preview_rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.label(app.language),
            egui::FontId::proportional(15.0),
            egui::Color32::WHITE,
        );
    }
}

/// Draws each connection as a horizontal S-shaped Bézier curve from the output
/// socket to the input socket. Connections to missing nodes are skipped.
fn paint_connections(
    connections: &HashMap<SocketRef, SocketRef>,
    nodes: &[NodeWidget],
    painter: &egui::Painter,
    origin: egui::Vec2,
) {
    for (to, from) in connections {
        let (Some(from_node), Some(to_node)) = (
            nodes.iter().find(|node| node.id == from.node_id),
            nodes.iter().find(|node| node.id == to.node_id),
        ) else {
            continue;
        };

        let start = socket_pos(from_node, from.side, from.index, origin);
        let end = socket_pos(to_node, to.side, to.index, origin);
        let handle = (end.x - start.x).abs() * 0.5;
        let control1 = start + egui::vec2(handle, 0.0);
        let control2 = end - egui::vec2(handle, 0.0);

        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [start, control1, control2, end],
            false,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(130, 200, 255)),
        ));
    }
}

/// Draws every node in order: body, title (`Kind id`), parameters, sockets with
/// their labels, and the delete button. All text is resolved in `language`.
fn paint_nodes(
    nodes: &[NodeWidget],
    painter: &egui::Painter,
    language: Language,
    origin: egui::Vec2,
) {
    for node in nodes {
        let rect = node_rect(node, origin);
        painter.rect_filled(rect, 10.0, node_color(node.config.kind));
        painter.rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(1.2, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.right_top() + egui::vec2(-6.0, 6.0),
            egui::Align2::RIGHT_TOP,
            format!("{} {}", node.config.kind.label(language), node.id),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
        for (index, param) in node.config.content.iter().enumerate() {
            painter.text(
                egui::pos2(rect.center().x, rect.top() + 50.0 + index as f32 * 22.0),
                egui::Align2::CENTER_CENTER,
                param.label(language),
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
        for (side, sockets) in [
            (SocketKind::Input, &node.config.inputs),
            (SocketKind::Output, &node.config.outputs),
        ] {
            for (index, label) in sockets.iter().enumerate() {
                let pos = socket_pos(node, side, index, origin);
                painter.circle_filled(pos, 7.0, egui::Color32::WHITE);
                let align = match side {
                    SocketKind::Input => egui::Align2::LEFT_CENTER,
                    SocketKind::Output => egui::Align2::RIGHT_CENTER,
                };
                let label_pos = pos
                    + egui::vec2(
                        if side == SocketKind::Input {
                            10.0
                        } else {
                            -10.0
                        },
                        0.0,
                    );
                painter.text(
                    label_pos,
                    align,
                    language.text(*label),
                    egui::FontId::proportional(10.0),
                    egui::Color32::WHITE,
                );
            }
        }

        let delete_rect = delete_button_rect(node, origin);
        painter.rect_filled(delete_rect, 4.0, egui::Color32::from_rgb(105, 48, 48));
        painter.text(
            delete_rect.center(),
            egui::Align2::CENTER_CENTER,
            "x",
            egui::FontId::proportional(15.0),
            egui::Color32::WHITE,
        );
    }
}

/// The node's bounds on screen.
fn node_rect(node: &NodeWidget, origin: egui::Vec2) -> egui::Rect {
    node.rect().translate(origin)
}

/// A socket's center on screen.
fn socket_pos(node: &NodeWidget, side: SocketKind, index: usize, origin: egui::Vec2) -> egui::Pos2 {
    node.socket_pos(side, index) + origin
}

/// A socket's 12×12 click area on screen, centered on the socket.
fn socket_rect(
    node: &NodeWidget,
    side: SocketKind,
    index: usize,
    origin: egui::Vec2,
) -> egui::Rect {
    egui::Rect::from_center_size(
        socket_pos(node, side, index, origin),
        egui::vec2(12.0, 12.0),
    )
}

/// The delete button's area on screen, in the node's top-left corner.
fn delete_button_rect(node: &NodeWidget, origin: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(
        node_rect(node, origin).left_top() + egui::vec2(6.0, 6.0),
        egui::vec2(DELETE_BUTTON_SIZE, DELETE_BUTTON_SIZE),
    )
}
