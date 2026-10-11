//! Central node canvas: draws nodes and connections and handles all node interaction.
//!
//! # Coordinates
//!
//! Nodes store **canvas coordinates** (`NodeWidget::pos`): `(0, 0)` is the top-left
//! of the scrollable content, and one canvas unit is one point at zoom 1.0. egui
//! works in **screen coordinates**. Each frame builds a [`View`] that maps between
//! the two, using where the canvas `(0, 0)` currently is on screen (`origin`) and
//! the toolbar zoom (`app.scale`):
//!
//! ```text
//! screen = canvas * scale + origin        canvas = (screen - origin) / scale
//! ```
//!
//! All geometry constants below are in canvas units unless noted otherwise, and
//! are scaled through the view when drawn, so everything zooms together. Anything
//! stored in the app uses canvas coordinates.
//!
//! # Scrolling and size
//!
//! The canvas is a [`egui::ScrollArea::both`] with solid, always-visible scroll
//! bars at the bottom and right. Its content is never smaller than the viewport
//! and grows to fit the furthest node plus [`CANVAS_MARGIN`] (see [`content_size`]),
//! so moving a node right or down extends the canvas. Nodes can't go above or left
//! of `(0, 0)`: a node dragged against that edge stops while the pointer moves on,
//! and follows again from the same grab point once the pointer comes back.
//!
//! # Interaction
//!
//! | Input                                   | Effect                                          |
//! |-----------------------------------------|-------------------------------------------------|
//! | drag node body                          | move the node (auto-scrolls near the edges)     |
//! | click or drag node body                 | bring the node to the front                     |
//! | primary click output socket             | select it as the pending source                 |
//! | primary click input socket              | connect the pending source to it, replacing     |
//! |                                         | the input's previous source                     |
//! | secondary click output socket           | remove every connection from that output        |
//! | secondary click input socket            | remove the connection into that input           |
//! | primary click `✕`                       | delete the node and its connections             |
//! | click empty canvas, or `Esc`            | clear the pending source                        |
//! | drop a palette button on the canvas     | add a node centered on the pointer              |
//! | click a palette button                  | add a node in the middle of the visible area    |
//!
//! Clicking an input always clears the selection. The connection itself goes
//! through `SignalForgeApp::connect`, which refuses edges that would create a cycle
//! (including a node feeding itself) and flags that for the debug panel.
//!
//! Feedback: the pending source gets an accent ring and a curve that follows the
//! pointer; hovered sockets and delete buttons light up and show a pointing-hand
//! cursor. Socket hit areas ([`SOCKET_HIT_SIZE`]) are larger than the drawn dot.
//!
//! Every interactive area uses an id derived from the node ID (not from creation
//! order), so ids stay stable when nodes are added, deleted or reordered.
//!
//! # Frame order
//!
//! 1. Allocate the content rect (also the click target for "empty canvas"),
//!    which fixes the [`View`] for this frame.
//! 2. Add palette requests (click or drop), then handle node input
//!    ([`handle_node_interactions`]). Widgets created later win overlapping clicks,
//!    so sockets and the delete button sit above their node body, and later nodes
//!    above earlier ones.
//! 3. Paint connections, then nodes, then the pending-source curve, then the
//!    palette drop preview on top.

use eframe::egui;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use super::routing::{self, Route};
use super::{
    ACCENT, DANGER, NodeConfig, NodeKind, PANEL_BG, TEXT, TEXT_DIM, node_color, place_new_node,
};
use crate::editor::{NodeWidget, SignalForgeApp, SocketKind, SocketRef, Text};

/// Corner radius of a node.
const NODE_ROUNDING: f32 = 8.0;
/// Width of the kind-colored node outline; the node body itself has the panel color.
const NODE_OUTLINE_WIDTH: f32 = 1.5;
/// Height of the tinted header band holding the title and the delete button.
const HEADER_HEIGHT: f32 = 28.0;
/// Opacity of the kind color tinting the node header.
const HEADER_TINT: f32 = 0.16;
/// Horizontal padding of the title from the node's left edge.
const TITLE_PADDING: f32 = 10.0;
/// Side of the square delete button in the node header's right end.
const DELETE_BUTTON_SIZE: f32 = 18.0;
/// Half the length of each stroke of the drawn delete cross.
const DELETE_CROSS_HALF: f32 = 3.5;
/// Distance of the delete button from the node's top-right corner.
const NODE_INSET: f32 = 5.0;
/// Radius of a socket dot.
const SOCKET_RADIUS: f32 = 5.0;
/// Extra radius of a hovered socket dot.
const SOCKET_HOVER_GROW: f32 = 1.5;
/// Side of a socket's square hit area; larger than the dot so it's easy to hit.
const SOCKET_HIT_SIZE: f32 = 20.0;
/// Gap between a socket dot's center and its label.
const SOCKET_LABEL_GAP: f32 = 11.0;
/// Width of a connection curve.
const CONNECTION_WIDTH: f32 = 2.0;
/// Shortest horizontal Bézier handle of a connection curve.
const CONNECTION_MIN_HANDLE: f32 = 20.0;
/// Spacing of the background dot grid; every other dot is a Snap-to-grid point.
const GRID_SPACING: f32 = 25.0;
/// The dot grid is skipped when its on-screen spacing would be denser than this.
const GRID_MIN_SCREEN_SPACING: f32 = 10.0;
/// Free space kept to the right of and below the furthest node, so the canvas can grow.
const CANVAS_MARGIN: f32 = 200.0;
/// Corner radius of routed (orthogonal) connections.
const ROUTE_CORNER_RADIUS: f32 = 10.0;
/// Points per rounded corner of a routed connection.
const ROUTE_CORNER_SEGMENTS: usize = 6;
/// Distance from the viewport edge at which dragging a node starts scrolling (screen points).
const EDGE_SCROLL_ZONE: f32 = 32.0;
/// Maximum edge-scroll speed, in screen points per second.
const EDGE_SCROLL_SPEED: f32 = 600.0;

/// Background grid dots.
const GRID_DOT: egui::Color32 = egui::Color32::from_rgb(28, 30, 37);

/// Maps canvas coordinates to screen coordinates for one frame.
#[derive(Clone, Copy)]
struct View {
    /// Screen position of the canvas origin `(0, 0)`.
    origin: egui::Vec2,
    /// Screen points per canvas unit.
    scale: f32,
}

impl View {
    fn pos(self, canvas: egui::Pos2) -> egui::Pos2 {
        (canvas.to_vec2() * self.scale + self.origin).to_pos2()
    }

    fn rect(self, canvas: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.pos(canvas.min), self.pos(canvas.max))
    }

    fn len(self, canvas: f32) -> f32 {
        canvas * self.scale
    }

    fn to_canvas(self, screen: egui::Pos2) -> egui::Pos2 {
        ((screen.to_vec2() - self.origin) / self.scale).to_pos2()
    }
}

/// What the pointer is over after interaction, for hover feedback when painting.
#[derive(Default)]
struct Hovered {
    socket: Option<SocketRef>,
    delete_button: Option<usize>,
}

/// Shows the canvas in the central panel, inside a scroll area.
pub(super) fn show(app: &mut SignalForgeApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        // Edge to edge: no padding or outline around the canvas.
        .frame(egui::Frame::NONE.fill(super::CANVAS_BG))
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
    let (canvas_rect, background) = ui.allocate_exact_size(
        content_size(&app.nodes, app.scale, viewport.size()),
        egui::Sense::click(),
    );
    let view = View {
        origin: canvas_rect.min.to_vec2(),
        scale: app.scale,
    };
    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| viewport.contains(*pos));

    if background.clicked() || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.pending_source = None;
    }
    add_requested_node(app, view, viewport);
    finish_palette_drop(app, ui, view, pointer);
    let hovered = handle_node_interactions(app, ui, view, viewport);

    let painter = ui.painter();
    paint_grid(painter, view, viewport);
    let routes = connection_routes(ui, app);
    paint_connections(&routes, painter, view);
    paint_nodes(app, painter, view, &hovered);
    paint_pending_connection(app, painter, view, ui.ctx().pointer_hover_pos());
    paint_palette_preview(app, ui.ctx(), painter, view, pointer);
}

/// Content fills at least the viewport and grows to fit every node plus a margin.
fn content_size(nodes: &[NodeWidget], scale: f32, viewport: egui::Vec2) -> egui::Vec2 {
    let extent = nodes.iter().fold(egui::Vec2::ZERO, |extent, node| {
        extent.max(node.rect().max.to_vec2() + egui::Vec2::splat(CANVAS_MARGIN))
    });
    (extent * scale).max(viewport)
}

/// Canvas rect of a node of `kind` dropped at screen `pointer`: centered on it, kept off negative coordinates.
///
/// Moved to the nearest free spot if that would overlap a node, exactly as
/// `add_node_at` will place it, so the preview shows the real landing spot.
fn drop_rect(kind: NodeKind, pointer: egui::Pos2, view: View, nodes: &[NodeWidget]) -> egui::Rect {
    let config = NodeConfig::for_kind(kind);
    let size = config.size();
    let pos = place_new_node(nodes, &config, view.to_canvas(pointer) - size * 0.5);
    egui::Rect::from_min_size(pos, size)
}

/// Adds the node requested by a palette click in the middle of the visible area
/// (or the nearest free spot, see `add_node_at`).
fn add_requested_node(app: &mut SignalForgeApp, view: View, viewport: egui::Rect) {
    let Some(kind) = app.requested_node.take() else {
        return;
    };
    let size = NodeConfig::for_kind(kind).size();
    app.add_node_at(kind, view.to_canvas(viewport.center()) - size * 0.5);
}

/// Adds the node when a palette drag is released over the canvas viewport
/// (`pointer` is `None` otherwise). Releases elsewhere are cleaned up by egui.
fn finish_palette_drop(
    app: &mut SignalForgeApp,
    ui: &egui::Ui,
    view: View,
    pointer: Option<egui::Pos2>,
) {
    if let Some(pos) = pointer
        && ui.input(|input| input.pointer.any_released())
        && let Some(kind) = egui::DragAndDrop::take_payload::<NodeKind>(ui.ctx())
    {
        app.add_node_at(*kind, drop_rect(*kind, pos, view, &app.nodes).min);
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
///
/// The node can't be pushed into another one: it slides towards the pointer and
/// stops at the first node in the way (see `placement::slide`). `nodes` holds the
/// rects of all nodes at the start of the frame, keyed by ID.
fn drag_node(
    ui: &egui::Ui,
    node: &mut NodeWidget,
    response: &egui::Response,
    view: View,
    viewport: egui::Rect,
    nodes: &[(usize, egui::Rect)],
) {
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    if response.drag_started() {
        // Measure the grab offset from where the button went down, before the drag threshold moved the pointer.
        let press = ui
            .input(|input| input.pointer.press_origin())
            .unwrap_or(pointer);
        let grab = view.to_canvas(press) - node.pos;
        ui.data_mut(|data| data.insert_temp(response.id, grab));
    }
    if response.dragged() {
        let grab: egui::Vec2 = ui
            .data(|data| data.get_temp(response.id))
            .unwrap_or_default();
        let obstacles: Vec<_> = nodes
            .iter()
            .filter(|(id, _)| *id != node.id)
            .map(|(_, rect)| *rect)
            .collect();
        node.pos = super::placement::slide(node.rect(), view.to_canvas(pointer) - grab, &obstacles);
        edge_scroll(ui, viewport, pointer);
    }
}

/// Registers the interactive areas of every node and applies this frame's clicks
/// and drags: moving, raising, selecting a source, connecting, disconnecting and
/// deleting. Returns what the pointer is over, for hover feedback.
///
/// Deleting and raising change the node list, so they are applied after the loop
/// (at most one of each per frame).
fn handle_node_interactions(
    app: &mut SignalForgeApp,
    ui: &mut egui::Ui,
    view: View,
    viewport: egui::Rect,
) -> Hovered {
    let mut hovered = Hovered::default();
    let node_rects: Vec<_> = app
        .nodes
        .iter()
        .map(|node| (node.id, node.rect()))
        .collect();
    let mut deleted_node_id = None;
    let mut raised_node_id = None;

    for node_index in 0..app.nodes.len() {
        let node = &mut app.nodes[node_index];
        // Ids are derived from the node ID, not from creation order, so they stay
        // stable when nodes are added, deleted or reordered. The grab offset of a
        // drag is stored under the body's id.
        let body_response = ui.interact(
            view.rect(node.rect()),
            ui.id().with(("canvas_node", node.id)),
            egui::Sense::click_and_drag(),
        );
        drag_node(ui, node, &body_response, view, viewport, &node_rects);
        if body_response.clicked() || body_response.drag_started() {
            raised_node_id = Some(node.id);
        }

        let socket_responses = |side: SocketKind, count: usize| -> Vec<egui::Response> {
            (0..count)
                .map(|index| {
                    ui.interact(
                        socket_rect(node, side, index, view),
                        ui.id().with(("canvas_socket", node.id, side, index)),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                })
                .collect()
        };
        let input_responses = socket_responses(SocketKind::Input, node.config.inputs.len());
        let output_responses = socket_responses(SocketKind::Output, node.config.outputs.len());
        let delete_response = ui
            .interact(
                view.rect(delete_button_rect(node)),
                ui.id().with(("canvas_node_delete", node.id)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(app.language.text(Text::DeleteNode));

        let node_id = node.id;
        for (side, responses) in [
            (SocketKind::Input, &input_responses),
            (SocketKind::Output, &output_responses),
        ] {
            if let Some(index) = responses.iter().position(egui::Response::hovered) {
                hovered.socket = Some(SocketRef {
                    node_id,
                    side,
                    index,
                });
            }
        }
        if delete_response.hovered() {
            hovered.delete_button = Some(node_id);
        }

        if delete_response.clicked_by(egui::PointerButton::Primary) {
            deleted_node_id = Some(node_id);
            continue;
        }

        if let Some(index) = output_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Primary))
        {
            app.pending_source = Some(SocketRef {
                node_id,
                side: SocketKind::Output,
                index,
            });
            app.connection_refused = false;
        }

        // `connect` borrows the whole app, so `node` can't be used past this point.
        if let Some(index) = input_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Primary))
            && let Some(from) = app.pending_source.take()
        {
            app.connect(
                from,
                SocketRef {
                    node_id,
                    side: SocketKind::Input,
                    index,
                },
            );
        }

        if let Some(index) = output_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Secondary))
        {
            app.connections.retain(|_, from| {
                !(from.node_id == node_id && from.side == SocketKind::Output && from.index == index)
            });
        }

        if let Some(index) = input_responses
            .iter()
            .position(|response| response.clicked_by(egui::PointerButton::Secondary))
        {
            app.connections.remove(&SocketRef {
                node_id,
                side: SocketKind::Input,
                index,
            });
        }
    }

    if let Some(node_id) = raised_node_id {
        app.raise_node(node_id);
    }
    if let Some(node_id) = deleted_node_id {
        app.remove_node(node_id);
    }
    hovered
}

fn paint_curve(painter: &egui::Painter, points: [egui::Pos2; 4], stroke: egui::Stroke) {
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        points,
        false,
        egui::Color32::TRANSPARENT,
        stroke,
    ));
}

/// Routes of all connections in canvas coordinates, keyed by input socket.
///
/// Routing runs a path search per blocked connection, so the result is cached in
/// egui memory and recomputed only when a node moves, is added or removed, or the
/// connections change.
fn connection_routes(ui: &egui::Ui, app: &SignalForgeApp) -> Arc<HashMap<SocketRef, Route>> {
    #[derive(Clone, Default)]
    struct Cache {
        key: u64,
        routes: Arc<HashMap<SocketRef, Route>>,
    }

    let mut connections: Vec<_> = app
        .connections
        .iter()
        .map(|(to, from)| (*to, *from))
        .collect();
    connections.sort_by_key(|(to, from)| (to.node_id, to.index, from.node_id, from.index));
    let mut hasher = DefaultHasher::new();
    for node in &app.nodes {
        let rect = node.rect();
        node.id.hash(&mut hasher);
        [rect.min.x, rect.min.y, rect.max.x, rect.max.y]
            .map(f32::to_bits)
            .hash(&mut hasher);
    }
    connections.hash(&mut hasher);
    let key = hasher.finish();

    let id = ui.id().with("canvas_connection_routes");
    if let Some(cache) = ui.data(|data| data.get_temp::<Cache>(id))
        && cache.key == key
    {
        return cache.routes;
    }

    let obstacles: Vec<_> = app.nodes.iter().map(NodeWidget::rect).collect();
    let socket_pos = |socket: SocketRef| {
        app.nodes
            .iter()
            .find(|node| node.id == socket.node_id)
            .map(|node| node.socket_pos(socket.side, socket.index))
    };
    let routes: HashMap<_, _> = connections
        .iter()
        .filter_map(|(to, from)| {
            let (start, end) = (socket_pos(*from)?, socket_pos(*to)?);
            Some((
                *to,
                routing::route(start, end, &obstacles, CONNECTION_MIN_HANDLE),
            ))
        })
        .collect();
    let routes = Arc::new(routes);
    ui.data_mut(|data| {
        data.insert_temp(
            id,
            Cache {
                key,
                routes: routes.clone(),
            },
        );
    });
    routes
}

/// Draws each connection along its route: the plain curve when nothing is in the
/// way, otherwise the orthogonal path around the nodes with rounded corners.
fn paint_connections(routes: &HashMap<SocketRef, Route>, painter: &egui::Painter, view: View) {
    let stroke = egui::Stroke::new(view.len(CONNECTION_WIDTH), ACCENT);
    for route in routes.values() {
        match route {
            Route::Curve(points) => paint_curve(painter, points.map(|p| view.pos(p)), stroke),
            Route::Path(points) => {
                let points: Vec<_> = points.iter().map(|p| view.pos(*p)).collect();
                painter.add(egui::Shape::line(
                    rounded_corners(&points, view.len(ROUTE_CORNER_RADIUS)),
                    stroke,
                ));
            }
        }
    }
}

/// Polyline through `points` with every corner replaced by a quadratic arc of
/// up to `radius` (less on short segments, so arcs never overlap).
fn rounded_corners(points: &[egui::Pos2], radius: f32) -> Vec<egui::Pos2> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    let mut result = vec![*first];
    for corner in points.windows(3) {
        let [before, at, after] = [corner[0], corner[1], corner[2]];
        let r = radius
            .min(before.distance(at) * 0.5)
            .min(at.distance(after) * 0.5);
        let entry = at + (before - at).normalized() * r;
        let exit = at + (after - at).normalized() * r;
        for k in 0..=ROUTE_CORNER_SEGMENTS {
            let t = k as f32 / ROUTE_CORNER_SEGMENTS as f32;
            let u = 1.0 - t;
            let point =
                entry.to_vec2() * (u * u) + at.to_vec2() * (2.0 * u * t) + exit.to_vec2() * (t * t);
            result.push(point.to_pos2());
        }
    }
    result.push(*last);
    result
}

/// Draws a translucent curve from the pending source socket to the pointer.
fn paint_pending_connection(
    app: &SignalForgeApp,
    painter: &egui::Painter,
    view: View,
    pointer: Option<egui::Pos2>,
) {
    let (Some(source), Some(pointer)) = (app.pending_source, pointer) else {
        return;
    };
    let Some(node) = app.nodes.iter().find(|node| node.id == source.node_id) else {
        return;
    };
    let start = view.pos(node.socket_pos(source.side, source.index));
    let stroke = egui::Stroke::new(view.len(CONNECTION_WIDTH), ACCENT.gamma_multiply(0.5));
    let curve = routing::bezier(start, pointer, view.len(CONNECTION_MIN_HANDLE));
    paint_curve(painter, curve, stroke);
}

/// Draws a dot grid over the visible part of the canvas, aligned to canvas
/// coordinates so it scrolls and zooms with the nodes. Skipped when zoomed out so
/// far that the dots would turn into noise.
fn paint_grid(painter: &egui::Painter, view: View, viewport: egui::Rect) {
    if view.len(GRID_SPACING) < GRID_MIN_SCREEN_SPACING {
        return;
    }
    let visible =
        egui::Rect::from_min_max(view.to_canvas(viewport.min), view.to_canvas(viewport.max));
    let radius = view.len(1.0).max(0.8);
    let first = (visible.min.to_vec2() / GRID_SPACING).floor() * GRID_SPACING;
    let mut y = first.y;
    while y <= visible.max.y {
        let mut x = first.x;
        while x <= visible.max.x {
            painter.circle_filled(view.pos(egui::pos2(x, y)), radius, GRID_DOT);
            x += GRID_SPACING;
        }
        y += GRID_SPACING;
    }
}

/// Draws every node in order: body (panel color), header tinted with
/// the kind color, kind-colored outline, title (`Kind #id`), parameters, sockets
/// with their labels, and the delete cross, with hover and pending-source
/// highlights. All text is resolved in the app's language and all sizes follow
/// the zoom.
fn paint_nodes(app: &SignalForgeApp, painter: &egui::Painter, view: View, hovered: &Hovered) {
    let language = app.language;
    let font = |size: f32| egui::FontId::proportional(view.len(size));
    for node in &app.nodes {
        let color = node_color(node.config.kind);
        let rect = view.rect(node.rect());
        let rounding = view.len(NODE_ROUNDING);

        painter.rect_filled(rect, rounding, PANEL_BG);
        let header =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), view.len(HEADER_HEIGHT)));
        let top_rounding = rounding.round().clamp(0.0, 255.0) as u8;
        painter.rect_filled(
            header,
            egui::CornerRadius {
                nw: top_rounding,
                ne: top_rounding,
                sw: 0,
                se: 0,
            },
            color.gamma_multiply(HEADER_TINT),
        );
        painter.hline(
            header.x_range(),
            header.bottom(),
            egui::Stroke::new(view.len(1.0), color.gamma_multiply(0.3)),
        );
        painter.rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(view.len(NODE_OUTLINE_WIDTH), color),
            egui::StrokeKind::Inside,
        );

        let title_rect = painter.text(
            egui::pos2(rect.left() + view.len(TITLE_PADDING), header.center().y),
            egui::Align2::LEFT_CENTER,
            node.config.kind.label(language),
            font(13.0),
            TEXT,
        );
        painter.text(
            egui::pos2(title_rect.right() + view.len(5.0), header.center().y),
            egui::Align2::LEFT_CENTER,
            format!("#{}", node.id),
            font(11.0),
            TEXT_DIM,
        );
        for (index, param) in node.config.content.iter().enumerate() {
            painter.text(
                view.pos(node.param_pos(index)),
                egui::Align2::CENTER_CENTER,
                param.label(language),
                font(12.0),
                TEXT_DIM,
            );
        }
        paint_sockets(app, node, painter, view, hovered);
        paint_delete_button(painter, view, node, hovered.delete_button == Some(node.id));
    }
}

/// Draws the delete button as a cross made of two lines, so it is always exactly
/// centered and crisp at any zoom. (A `✕` glyph comes from egui's fallback emoji
/// font, whose metrics differ from the main font, so it renders off-center.)
/// Dim at rest; a red rounded square with a white cross when hovered.
fn paint_delete_button(painter: &egui::Painter, view: View, node: &NodeWidget, hovered: bool) {
    let rect = view.rect(delete_button_rect(node));
    let cross_color = if hovered {
        painter.rect_filled(rect, view.len(4.0), DANGER.gamma_multiply(0.85));
        egui::Color32::WHITE
    } else {
        TEXT_DIM
    };
    let stroke = egui::Stroke::new(view.len(1.5), cross_color);
    let half = view.len(DELETE_CROSS_HALF);
    let center = rect.center();
    painter.line_segment(
        [
            center + egui::vec2(-half, -half),
            center + egui::vec2(half, half),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(-half, half),
            center + egui::vec2(half, -half),
        ],
        stroke,
    );
}

/// Draws a node's socket dots and labels. Dots have the kind color with a ring
/// of the body color, so they read as cut into the outline; a hovered socket
/// grows and turns accent-colored, and the pending source gets an accent ring.
fn paint_sockets(
    app: &SignalForgeApp,
    node: &NodeWidget,
    painter: &egui::Painter,
    view: View,
    hovered: &Hovered,
) {
    let color = node_color(node.config.kind);
    for (side, sockets) in [
        (SocketKind::Input, &node.config.inputs),
        (SocketKind::Output, &node.config.outputs),
    ] {
        for (index, label) in sockets.iter().enumerate() {
            let socket = SocketRef {
                node_id: node.id,
                side,
                index,
            };
            let pos = view.pos(node.socket_pos(side, index));
            let (radius, fill) = if hovered.socket == Some(socket) {
                (view.len(SOCKET_RADIUS + SOCKET_HOVER_GROW), ACCENT)
            } else {
                (view.len(SOCKET_RADIUS), color)
            };
            painter.circle(
                pos,
                radius,
                fill,
                egui::Stroke::new(view.len(2.0), PANEL_BG),
            );
            if app.pending_source == Some(socket) {
                painter.circle_stroke(
                    pos,
                    radius + view.len(4.0),
                    egui::Stroke::new(view.len(1.5), ACCENT),
                );
            }

            let (align, direction) = match side {
                SocketKind::Input => (egui::Align2::LEFT_CENTER, 1.0),
                SocketKind::Output => (egui::Align2::RIGHT_CENTER, -1.0),
            };
            painter.text(
                pos + egui::vec2(direction * view.len(SOCKET_LABEL_GAP), 0.0),
                align,
                app.language.text(*label),
                egui::FontId::proportional(view.len(11.0)),
                TEXT_DIM,
            );
        }
    }
}

/// Draws a translucent node of the dragged palette kind exactly where it would be dropped.
fn paint_palette_preview(
    app: &SignalForgeApp,
    ctx: &egui::Context,
    painter: &egui::Painter,
    view: View,
    pointer: Option<egui::Pos2>,
) {
    let (Some(kind), Some(pos)) = (egui::DragAndDrop::payload::<NodeKind>(ctx), pointer) else {
        return;
    };
    let color = node_color(*kind);
    let preview_rect = view.rect(drop_rect(*kind, pos, view, &app.nodes));
    let rounding = view.len(NODE_ROUNDING);
    painter.rect_filled(preview_rect, rounding, color.gamma_multiply(0.08));
    painter.rect_stroke(
        preview_rect,
        rounding,
        egui::Stroke::new(view.len(NODE_OUTLINE_WIDTH), color.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        preview_rect.center(),
        egui::Align2::CENTER_CENTER,
        kind.label(app.language),
        egui::FontId::proportional(view.len(14.0)),
        TEXT,
    );
}

/// A socket's square hit area on screen, centered on the socket.
fn socket_rect(node: &NodeWidget, side: SocketKind, index: usize, view: View) -> egui::Rect {
    egui::Rect::from_center_size(
        view.pos(node.socket_pos(side, index)),
        egui::Vec2::splat(view.len(SOCKET_HIT_SIZE)),
    )
}

/// The delete button's area in canvas coordinates: at the right end of the
/// header, vertically centered in it.
fn delete_button_rect(node: &NodeWidget) -> egui::Rect {
    let rect = node.rect();
    egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - NODE_INSET - DELETE_BUTTON_SIZE * 0.5,
            rect.top() + HEADER_HEIGHT * 0.5,
        ),
        egui::Vec2::splat(DELETE_BUTTON_SIZE),
    )
}
