//! Routing connections around nodes.
//!
//! [`route`] first tries the plain S-shaped Bézier curve from the output to the
//! input. If that curve passes through any node, it looks for an orthogonal path
//! around the nodes instead:
//!
//! 1. Leave the output with a horizontal stub of [`STUB`] and enter the input the
//!    same way, so connections always run out of the right edge and into the left.
//! 2. Search between the two stub ends on a grid with A*. Cells closer than
//!    [`CLEARANCE`] to a node are blocked; every turn costs extra, so paths have
//!    as few bends as possible; arriving at the input other than left-to-right
//!    costs a turn too.
//! 3. Merge collinear cells into corner points. The canvas draws the result with
//!    rounded corners.
//!
//! The grid is aligned so both stub ends are exactly grid points: the cell size is
//! adjusted per connection (close to [`CELL`]) so the distance between them is a
//! whole number of cells. That keeps the final segments straight, without jogs.
//!
//! If no path exists inside the search area, the plain curve is used. Everything
//! is in canvas coordinates.

use eframe::egui::{Pos2, Rect, Vec2, vec2};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Length of the horizontal run out of an output and into an input.
const STUB: f32 = 20.0;
/// Distance a routed path keeps from every node.
const CLEARANCE: f32 = 12.0;
/// Nominal grid cell size of the path search.
const CELL: f32 = 10.0;
/// How far beyond the two endpoints the search may go around obstacles.
const SEARCH_MARGIN: f32 = 240.0;
/// Upper bound on grid cells per search, to keep the cost of one route bounded.
const MAX_CELLS: usize = 120_000;
/// Points sampled along the Bézier curve when testing it against nodes.
const CURVE_SAMPLES: usize = 48;
/// Path cost of one grid step.
const STEP_COST: u32 = 10;
/// Extra path cost of a 90° turn.
const TURN_COST: u32 = 50;

/// How a connection is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Route {
    /// Cubic Bézier control points: start, two handles, end.
    Curve([Pos2; 4]),
    /// Orthogonal polyline from the output to the input, corners only.
    Path(Vec<Pos2>),
}

/// Control points of the plain connection curve from `start` (an output) to `end`
/// (an input): horizontal handles at least `min_handle` long, so the curve leaves
/// the output rightwards and enters the input from the left.
pub(super) fn bezier(start: Pos2, end: Pos2, min_handle: f32) -> [Pos2; 4] {
    let handle = ((end.x - start.x).abs() * 0.5).max(min_handle);
    [
        start,
        start + vec2(handle, 0.0),
        end - vec2(handle, 0.0),
        end,
    ]
}

/// How to draw the connection from `start` to `end`, avoiding `obstacles` (all
/// node rects, including the two connected nodes).
pub(super) fn route(start: Pos2, end: Pos2, obstacles: &[Rect], min_handle: f32) -> Route {
    let curve = bezier(start, end, min_handle);
    if !curve_hits(&curve, obstacles) {
        return Route::Curve(curve);
    }
    find_path(start, end, obstacles).map_or(Route::Curve(curve), Route::Path)
}

fn cubic(points: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let [p0, p1, p2, p3] = points.map(Pos2::to_vec2);
    (p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t))
        .to_pos2()
}

/// Whether the curve passes through a node. The ends are skipped: they sit on
/// the connected nodes' edges by design.
fn curve_hits(curve: &[Pos2; 4], obstacles: &[Rect]) -> bool {
    (1..CURVE_SAMPLES)
        .map(|i| i as f32 / CURVE_SAMPLES as f32)
        .filter(|t| (0.04..=0.96).contains(t))
        .map(|t| cubic(curve, t))
        .any(|point| {
            obstacles.iter().any(|rect| {
                point.x > rect.min.x
                    && point.x < rect.max.x
                    && point.y > rect.min.y
                    && point.y < rect.max.y
            })
        })
}

/// Grid directions: right, left, down, up.
const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const RIGHT: usize = 0;

/// Orthogonal path around `obstacles`, as corner points from `start` to `end`;
/// `None` if the search area has no way through.
fn find_path(start: Pos2, end: Pos2, obstacles: &[Rect]) -> Option<Vec<Pos2>> {
    let from = start + vec2(STUB, 0.0);
    let to = end - vec2(STUB, 0.0);
    let grid = Grid::new(from, to, obstacles)?;
    let cells = grid.search()?;

    let mut points = vec![start];
    points.extend(cells.into_iter().map(|cell| grid.pos(cell)));
    points.push(end);
    Some(corners(points))
}

/// Drops points that lie on a straight line between their neighbors.
fn corners(points: Vec<Pos2>) -> Vec<Pos2> {
    let mut result: Vec<Pos2> = Vec::with_capacity(points.len());
    for point in points {
        if result.last() == Some(&point) {
            continue;
        }
        if let [.., a, b] = result[..] {
            let collinear = (a.x == b.x && b.x == point.x) || (a.y == b.y && b.y == point.y);
            if collinear {
                result.pop();
            }
        }
        result.push(point);
    }
    result
}

/// Search grid for one connection. Grid point `(i, j)` is at `origin + (i, j) * cell`;
/// `origin` is the output's stub end, and the input's stub end is also a grid point.
struct Grid {
    origin: Pos2,
    cell: Vec2,
    /// Index range of grid points, inclusive.
    min: (i32, i32),
    columns: usize,
    rows: usize,
    blocked: Vec<bool>,
    goal: (i32, i32),
}

impl Grid {
    fn new(from: Pos2, to: Pos2, obstacles: &[Rect]) -> Option<Self> {
        let delta = to - from;
        let cell_size = |distance: f32| {
            let steps = (distance / CELL).round();
            if steps == 0.0 {
                CELL
            } else {
                (distance / steps).abs()
            }
        };
        let cell = vec2(cell_size(delta.x), cell_size(delta.y));
        let goal = (
            (delta.x / cell.x).round() as i32,
            (delta.y / cell.y).round() as i32,
        );

        // Search area: around both stub ends, but not into negative coordinates
        // (unless an endpoint itself is there).
        let floor = from.min(to).min(Pos2::ZERO);
        let lo = (from.min(to) - Vec2::splat(SEARCH_MARGIN)).max(floor);
        let hi = from.max(to) + Vec2::splat(SEARCH_MARGIN);
        let min = (
            ((lo.x - from.x) / cell.x).floor() as i32,
            ((lo.y - from.y) / cell.y).floor() as i32,
        );
        let max = (
            ((hi.x - from.x) / cell.x).ceil() as i32,
            ((hi.y - from.y) / cell.y).ceil() as i32,
        );
        let columns = (max.0 - min.0 + 1) as usize;
        let rows = (max.1 - min.1 + 1) as usize;
        if columns * rows > MAX_CELLS {
            return None;
        }

        let mut grid = Self {
            origin: from,
            cell,
            min,
            columns,
            rows,
            blocked: vec![false; columns * rows],
            goal,
        };
        for obstacle in obstacles.iter().map(|rect| rect.expand(CLEARANCE)) {
            let i0 = ((obstacle.min.x - from.x) / cell.x).ceil() as i32;
            let i1 = ((obstacle.max.x - from.x) / cell.x).floor() as i32;
            let j0 = ((obstacle.min.y - from.y) / cell.y).ceil() as i32;
            let j1 = ((obstacle.max.y - from.y) / cell.y).floor() as i32;
            for j in j0.max(min.1)..=j1.min(max.1) {
                for i in i0.max(min.0)..=i1.min(max.0) {
                    let index = grid.index((i, j)).expect("clamped to the grid");
                    grid.blocked[index] = true;
                }
            }
        }
        // The stub ends sit right next to their nodes; they must stay reachable.
        for end in [(0, 0), goal] {
            if let Some(index) = grid.index(end) {
                grid.blocked[index] = false;
            }
        }
        Some(grid)
    }

    fn index(&self, (i, j): (i32, i32)) -> Option<usize> {
        let x = usize::try_from(i - self.min.0).ok()?;
        let y = usize::try_from(j - self.min.1).ok()?;
        (x < self.columns && y < self.rows).then_some(y * self.columns + x)
    }

    fn cell_of(&self, index: usize) -> (i32, i32) {
        (
            (index % self.columns) as i32 + self.min.0,
            (index / self.columns) as i32 + self.min.1,
        )
    }

    fn pos(&self, (i, j): (i32, i32)) -> Pos2 {
        self.origin + vec2(i as f32 * self.cell.x, j as f32 * self.cell.y)
    }

    /// A* from `(0, 0)` to `goal` over (cell, direction) states, with turn costs.
    /// Starts heading right (out of the output); arriving at the goal heading
    /// anywhere but right costs a turn. Returns the cells along the path.
    fn search(&self) -> Option<Vec<(i32, i32)>> {
        let start = self.index((0, 0))?;
        let goal = self.index(self.goal)?;
        let state = |index: usize, direction: usize| index * 4 + direction;
        let heuristic =
            |(i, j): (i32, i32)| (i.abs_diff(self.goal.0) + j.abs_diff(self.goal.1)) * STEP_COST;

        let mut cost = vec![u32::MAX; self.blocked.len() * 4];
        let mut came_from = vec![usize::MAX; self.blocked.len() * 4];
        let mut open = BinaryHeap::new();
        cost[state(start, RIGHT)] = 0;
        open.push(Reverse((heuristic((0, 0)), state(start, RIGHT))));

        while let Some(Reverse((_, current))) = open.pop() {
            let (index, direction) = (current / 4, current % 4);
            if index == goal {
                return Some(self.cells_to(current, &came_from));
            }
            let cell = self.cell_of(index);
            for (next_direction, (di, dj)) in DIRECTIONS.iter().enumerate() {
                // Never reverse on the spot.
                if next_direction == direction ^ 1 {
                    continue;
                }
                let next_cell = (cell.0 + di, cell.1 + dj);
                let Some(next_index) = self.index(next_cell) else {
                    continue;
                };
                if self.blocked[next_index] {
                    continue;
                }
                let mut step = STEP_COST;
                if next_direction != direction {
                    step += TURN_COST;
                }
                if next_index == goal && next_direction != RIGHT {
                    step += TURN_COST;
                }
                let next = state(next_index, next_direction);
                let next_cost = cost[current] + step;
                if next_cost < cost[next] {
                    cost[next] = next_cost;
                    came_from[next] = current;
                    open.push(Reverse((next_cost + heuristic(next_cell), next)));
                }
            }
        }
        None
    }

    fn cells_to(&self, mut state: usize, came_from: &[usize]) -> Vec<(i32, i32)> {
        let mut cells = vec![self.cell_of(state / 4)];
        while came_from[state] != usize::MAX {
            state = came_from[state];
            cells.push(self.cell_of(state / 4));
        }
        cells.reverse();
        cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn node(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(160.0, 90.0))
    }

    /// Whether any point along the polyline is strictly inside `rect`.
    fn path_hits(points: &[Pos2], rect: Rect) -> bool {
        points.windows(2).any(|segment| {
            (0..=20).any(|k| {
                let point = segment[0].lerp(segment[1], k as f32 / 20.0);
                point.x > rect.min.x
                    && point.x < rect.max.x
                    && point.y > rect.min.y
                    && point.y < rect.max.y
            })
        })
    }

    #[test]
    fn clear_connections_stay_curves() {
        let (a, b) = (node(0.0, 0.0), node(400.0, 0.0));
        let route = route(pos2(160.0, 45.0), pos2(400.0, 45.0), &[a, b], 20.0);
        assert!(matches!(route, Route::Curve(_)));
    }

    #[test]
    fn connections_go_around_a_node_in_the_way() {
        let (a, blocker, b) = (node(0.0, 0.0), node(250.0, 0.0), node(500.0, 0.0));
        let (start, end) = (pos2(160.0, 45.0), pos2(500.0, 45.0));

        let Route::Path(points) = route(start, end, &[a, blocker, b], 20.0) else {
            panic!("expected a routed path");
        };

        assert_eq!(points.first(), Some(&start));
        assert_eq!(points.last(), Some(&end));
        assert!(!path_hits(&points, blocker));
        // Orthogonal: every segment is horizontal or vertical.
        assert!(
            points
                .windows(2)
                .all(|s| s[0].x == s[1].x || s[0].y == s[1].y)
        );
        // Leaves rightwards and enters from the left.
        assert!(points[1].x > start.x && points[1].y == start.y);
        assert!(points[points.len() - 2].x < end.x);
    }

    #[test]
    fn backward_connections_loop_around_both_nodes() {
        // Input left of the output: the curve would cut through both nodes.
        let (a, b) = (node(400.0, 200.0), node(0.0, 200.0));
        let (start, end) = (pos2(560.0, 245.0), pos2(0.0, 245.0));

        let Route::Path(points) = route(start, end, &[a, b], 20.0) else {
            panic!("expected a routed path");
        };

        assert!(!path_hits(&points, a));
        assert!(!path_hits(&points, b));
    }
}
