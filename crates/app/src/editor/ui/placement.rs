//! Keeping nodes from overlapping.
//!
//! Nodes always keep at least [`NODE_GAP`] between each other. Two strategies:
//!
//! - [`slide`] — for dragging: the node moves towards the target and stops at the
//!   first node in its way (swept per axis, so a fast pointer can't tunnel through).
//!   It then slides along that node as the pointer keeps moving.
//! - [`free_pos`] — for adding and snapping: the nearest free position to the
//!   wanted one, searched outwards on a grid.
//!
//! Everything is in canvas coordinates, and positions never go negative.

use eframe::egui;

use super::{NodeConfig, NodeWidget};

/// Minimum free space between two nodes, in canvas units.
pub(super) const NODE_GAP: f32 = 10.0;
/// Grid step used when looking for a free spot for a new node.
const PLACEMENT_STEP: f32 = 10.0;
/// Rings searched by [`free_pos`] before giving up and returning the wanted position.
const MAX_SEARCH_RINGS: i32 = 200;

/// Whether `a` and `b` overlap or are closer than [`NODE_GAP`]. Touching at
/// exactly the gap is allowed.
fn too_close(a: egui::Rect, b: egui::Rect) -> bool {
    let b = b.expand(NODE_GAP);
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// Whether a node at `rect` would be too close to any of `obstacles`.
pub(super) fn blocked(rect: egui::Rect, obstacles: &[egui::Rect]) -> bool {
    obstacles.iter().any(|obstacle| too_close(rect, *obstacle))
}

/// Moves a node from `current` towards `target` (its new top-left corner), stopping
/// at the first obstacle in the way on each axis.
///
/// Obstacles the node already overlaps are ignored, so an overlapping node (from an
/// older layout) can always be dragged free. Both axis orders (x then y, y then x)
/// are tried and the result closer to `target` wins, so the node slides around
/// corners in the direction the pointer went.
pub(super) fn slide(
    current: egui::Rect,
    target: egui::Pos2,
    obstacles: &[egui::Rect],
) -> egui::Pos2 {
    let target = target.max(egui::Pos2::ZERO);
    let obstacles: Vec<_> = obstacles
        .iter()
        .copied()
        .filter(|obstacle| !too_close(current, *obstacle))
        .collect();
    let x_first = sweep_y(sweep_x(current, target.x, &obstacles), target.y, &obstacles);
    let y_first = sweep_x(sweep_y(current, target.y, &obstacles), target.x, &obstacles);
    if x_first.min.distance_sq(target) <= y_first.min.distance_sq(target) {
        x_first.min
    } else {
        y_first.min
    }
}

/// Moves `rect` horizontally to `x` (its new left edge), stopping before obstacles.
fn sweep_x(rect: egui::Rect, x: f32, obstacles: &[egui::Rect]) -> egui::Rect {
    let mut x = x;
    for obstacle in obstacles.iter().map(|obstacle| obstacle.expand(NODE_GAP)) {
        let rows_overlap = rect.min.y < obstacle.max.y && obstacle.min.y < rect.max.y;
        if !rows_overlap {
            continue;
        }
        if x > rect.min.x && obstacle.min.x >= rect.max.x {
            x = x.min(obstacle.min.x - rect.width());
        } else if x < rect.min.x && obstacle.max.x <= rect.min.x {
            x = x.max(obstacle.max.x);
        }
    }
    rect.translate(egui::vec2(x - rect.min.x, 0.0))
}

/// Moves `rect` vertically to `y` (its new top edge), stopping before obstacles.
fn sweep_y(rect: egui::Rect, y: f32, obstacles: &[egui::Rect]) -> egui::Rect {
    let mut y = y;
    for obstacle in obstacles.iter().map(|obstacle| obstacle.expand(NODE_GAP)) {
        let columns_overlap = rect.min.x < obstacle.max.x && obstacle.min.x < rect.max.x;
        if !columns_overlap {
            continue;
        }
        if y > rect.min.y && obstacle.min.y >= rect.max.y {
            y = y.min(obstacle.min.y - rect.height());
        } else if y < rect.min.y && obstacle.max.y <= rect.min.y {
            y = y.max(obstacle.max.y);
        }
    }
    rect.translate(egui::vec2(0.0, y - rect.min.y))
}

/// The free top-left position closest to `wanted` for a node of `size`, on a
/// grid of `step` around `wanted` (so a grid-aligned `wanted` stays aligned).
///
/// Searches square rings of growing radius and takes the closest free candidate
/// of the first ring that has one. Falls back to `wanted` if nothing is free
/// within [`MAX_SEARCH_RINGS`] rings.
pub(super) fn free_pos(
    wanted: egui::Pos2,
    size: egui::Vec2,
    obstacles: &[egui::Rect],
    step: f32,
) -> egui::Pos2 {
    let wanted = wanted.max(egui::Pos2::ZERO);
    let is_free = |pos: egui::Pos2| !blocked(egui::Rect::from_min_size(pos, size), obstacles);
    if is_free(wanted) {
        return wanted;
    }
    for ring in 1..=MAX_SEARCH_RINGS {
        let best = ring_offsets(ring)
            .map(|(dx, dy)| wanted + egui::vec2(dx as f32, dy as f32) * step)
            .filter(|pos| pos.x >= 0.0 && pos.y >= 0.0 && is_free(*pos))
            .min_by(|a, b| a.distance_sq(wanted).total_cmp(&b.distance_sq(wanted)));
        if let Some(pos) = best {
            return pos;
        }
    }
    wanted
}

/// Where a new node with `config` wanted at `wanted` actually goes: the nearest
/// spot (on a [`PLACEMENT_STEP`] grid around `wanted`) that keeps clear of `nodes`.
pub(in crate::editor) fn place_new_node(
    nodes: &[NodeWidget],
    config: &NodeConfig,
    wanted: egui::Pos2,
) -> egui::Pos2 {
    let obstacles: Vec<_> = nodes.iter().map(NodeWidget::rect).collect();
    free_pos(wanted, config.size(), &obstacles, PLACEMENT_STEP)
}

/// Grid offsets on the square ring at Chebyshev distance `ring` from the center.
fn ring_offsets(ring: i32) -> impl Iterator<Item = (i32, i32)> {
    (-ring..=ring).flat_map(move |dx| {
        (-ring..=ring)
            .filter(move |dy| dx.abs() == ring || dy.abs() == ring)
            .map(move |dy| (dx, dy))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(100.0, 50.0))
    }

    #[test]
    fn slide_stops_at_neighbor_without_tunneling() {
        let obstacle = rect(300.0, 0.0);
        // Jump far past the obstacle in one step: must stop in front of it.
        let pos = slide(rect(0.0, 0.0), egui::pos2(1000.0, 0.0), &[obstacle]);

        assert_eq!(pos, egui::pos2(300.0 - NODE_GAP - 100.0, 0.0));
        assert!(!blocked(
            egui::Rect::from_min_size(pos, rect(0.0, 0.0).size()),
            &[obstacle]
        ));
    }

    #[test]
    fn slide_moves_freely_along_a_blocked_axis() {
        let obstacle = rect(150.0, 0.0);
        let pos = slide(rect(0.0, 0.0), egui::pos2(200.0, 200.0), &[obstacle]);

        // Can't go through on the first row, but ends up below it at the target.
        assert_eq!(pos, egui::pos2(200.0, 200.0));
    }

    #[test]
    fn slide_lets_an_overlapping_node_escape() {
        let obstacle = rect(20.0, 20.0);
        let pos = slide(rect(0.0, 0.0), egui::pos2(0.0, 300.0), &[obstacle]);

        assert_eq!(pos, egui::pos2(0.0, 300.0));
    }

    #[test]
    fn free_pos_finds_the_nearest_gap() {
        let obstacles = [rect(0.0, 0.0)];
        let size = egui::vec2(100.0, 50.0);

        let pos = free_pos(egui::pos2(0.0, 0.0), size, &obstacles, 10.0);

        assert!(!blocked(egui::Rect::from_min_size(pos, size), &obstacles));
        assert!(pos.x >= 0.0 && pos.y >= 0.0);
        // Nearest free spot is just below (50 + gap), closer than to the right (100 + gap).
        assert_eq!(pos, egui::pos2(0.0, 60.0));
    }

    #[test]
    fn free_pos_keeps_a_free_position() {
        let pos = free_pos(
            egui::pos2(500.0, 500.0),
            egui::vec2(100.0, 50.0),
            &[rect(0.0, 0.0)],
            10.0,
        );
        assert_eq!(pos, egui::pos2(500.0, 500.0));
    }
}
